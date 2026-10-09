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

2. **Indexes, fused.** Every memory is indexed twice: stemmed
   (`porter unicode61`) so that *migrating* finds *migration*, and unstemmed
   (`unicode61`) so that an exact word beats a stem of it. A third, stemmed by
   the language the memory was written in, joins them where a language has a
   stemmer (§16). The result lists are merged by reciprocal rank fusion — a
   memory is worth `1 / (60 + place)` in each list it appears in, and the sum
   orders the answer.

   **A pin/recency/stability rerank was measured and is not taken.** Engram
   orders by `bm25 × (1 + 0.10·pinned + 0.06·recency + 0.04·stability)` and
   publishes no measurement of its own, so the factor was put to this crate's
   own ranking statement rather than copied. Both orderings — the shipped one,
   and the same statement with that factor as its sort key — ran over one copy
   of a real 5,273-memory store, 300 questions per draw over four seeds, through
   `tools/retrieval`. The factor is neutral on bodies and plainly worse on
   titles: mean MRR 0.9706 against 0.9722 on bodies, inside the seed-to-seed
   spread, and 0.8666 against 0.8042 on titles, with top-1 falling from 81.9% to
   73.4% and the held-out draw repeating the loss (0.8718 against 0.8075). A
   sort key applied to a ranking that already knows what matched lifts a pinned
   or recent memory above the memory that matched best, and on a title-shaped
   question the best match is usually the answer. Recorded here so the factor is
   not re-proposed without a measurement that contradicts this one.

3. **Six lexical stages, in order, stopping at the first that answers; a
   seventh reads meaning, and is §15.**
   1. every word must match;
   2. failing that, every word as a prefix — a word somebody half-remembers;
   3. failing that, every word as a substring of the title — a fragment from
      the middle of a word no prefix can reach;
   4. failing that, every word the index does not hold read as the nearest word
      it does — a typo, one or two edits away, before the question is loosened;
   5. failing that, all but one of them;
   6. failing that, any of them, kept only above a floor relative to the median
      rank of what came back.
   Requiring every word is the right first answer, but it fails completely
   rather than partially: one word the store has never seen takes the whole
   question down. Measured over two hundred questions drawn from the titles of a
   real 2,643-memory store, that happened to 4% of short questions and 12% of
   long ones, and the widened retry found the memory every time, at rank one
   every time.

   **The prefix and substring stages before the widening are for a word
   somebody only half-remembers.** `pgxpo` for `pgxpool` is a prefix the strict
   pass cannot match, because it wants the whole token; the second stage opens
   every word at its end, a smaller claim than the widening's dropping one.
   `telemetr` for `OpenTelemetry` is not the beginning of the word, so the
   third stage asks whether the title *contains* each word, and only the title:
   a title is where an identifier lives and a fraction of a memory's size, while
   over bodies the scan would read everything the store holds on every question
   that reached this far. Both mark their results `partial`, the way the
   widening does, because both matched a fragment rather than the whole word.

   The fourth stage is for a word that is *wrong* rather than unfinished, which
   neither a prefix nor a substring can answer: `conection` does not begin a
   word and is not inside one. It is requirement 13's subject and runs before
   the widening because reading a word as the word it meant is a smaller claim
   than dropping a word and hoping the rest is enough.

   The indexed way to ask the same question is a trigram index, and it was
   built and measured before the title scan was written: `tokenize = 'trigram'`
   over title and content added 18.5 MB to a 9.3 MB corpus — twice the text —
   for a stage that runs only once every indexed stage has already found
   nothing. A scan of the titles costs no bytes and no write time; measured on
   a 5,243-memory real store it adds about 3 ms to the query, against 0 ms for
   the strict pass, which is why no new index is added and no save latency
   changes. Over the benchmark's partial-word set the two stages take MRR from
   0.231 to 0.923 — twelve of thirteen at rank one — and the one left is a
   query whose target memory never contains the word asked for. The
   head-to-head against Engram on the same set is
   [`docs/comparison-with-engram.md`](../../docs/comparison-with-engram.md);
   its numbers are measured per run and are not restated here.

   **The final stage's floor is dimensionless, and that is not a defect to be
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

4. **A relaxed answer says it is relaxed, and an empty one says why it is
   empty.** Results from any stage below the strict pass — one that matched less
   than every word whole — carry `partial: true`, and the answer carries a hint
   saying so. The correction stage is the exception: it rebuilt the question
   rather than loosening it, so its rows match every word whole and are not
   marked `partial`; instead the answer names every substitution, "searched for
   X instead of Y", and that sentence is what tells the reader the words
   changed. A row the semantic stage added carries `semantic: true` instead,
   and the answer says so once — §15. An empty answer has two possible
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
   answer. A query that names the type (`type: session_summary`) asked for one,
   so every stage skips the exclusion for it and `visible_observations` narrows
   to it; that is the only way a summary is found from a loosened question.

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

9. **Deleted memories are never returned, and neither are superseded ones.** A
   deletion excludes the row ([`memory-model.md`](memory-model.md) §8); a judged
   `supersedes` excludes the memory it points at, because it is out of date
   rather than gone and reversing the verdict brings it back
   ([`memory-model.md`](memory-model.md) §13). A pending relation hides nothing.
   The exclusion is one clause, `not_superseded`, added to every stage rather
   than restated, so a stage cannot list a memory the context beside it has
   hidden.

10. **A disjunction is bounded; a conjunction is not, in terms.** Any stage that
    joins a query's words with `OR` — the final stage above, `mode: any` from the
    tool and the command line, and the per-prompt hint — takes at most the first
    thirty-two, from one constant that all of them read. A conjunction is not cut
    by term count: two hundred terms joined by `AND` match almost nothing and
    cost almost nothing to find out, while cutting them would answer a different
    question from the one somebody quoted. What is bounded is the raw query they
    are built from, by bytes, before tokenising — see §14.

   The bound was the hint's alone. The final stage documents itself as running
   the hint's own rule and then built its terms with the unbounded helper, so a
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

13. **A word the store has never held is read as the word it was meant to be,
    and every substitution is named.** A term that matches nothing in the
    stemmed index is a candidate. A term the index holds is never changed —
    correcting it would answer a different question from the one asked — and
    neither is an inflected word the stemmer already reaches (`limitting` for
    `limit`), which is why the decision is made against the *stemmed* indexes —
    `porter` and, where §16 applies, Snowball — and not the vocabulary. The replacement comes from the *unstemmed* vocabulary,
    so the word put back into the query is a word somebody could have written.
    The nearest word within an edit budget is chosen — one edit at five
    characters or fewer, two above that — with ties broken by the word the index
    holds in more memories and then by the lexicographically smaller one, so the
    answer does not depend on the order SQLite returns rows in.

    Every unknown word has to be placed, or none is. A conjunction that still
    carries one unknown word fails exactly as the original did, so a partial
    correction would read the vocabulary and report a substitution that changed
    no answer.

    The stage runs only when the strict pass came back empty, after the fragment
    stages and before the widening. A query that already answered never builds
    the vocabulary, and that is the guard: the temp table is created on this
    path and nowhere else, so a search which answers leaves it absent. The
    vocabulary is a per-connection TEMP `fts5vocab` over `observations_exact`,
    read directly from the index, so there is no migration and no
    `SCHEMA_VERSION` bump; an unreadable one is not an error and the search
    falls through as it did before the stage existed. Both surfaces share one
    sentence, built in `corrected_terms_hint`.

    It is the stage that pays for the typo set: measured on
    `tools/engram-bench`, MRR on the `typo` kind moves from 0.615 to 1.000 —
    thirteen of thirteen at rank one — and overall from 0.784 to 0.835, with no
    kind down. What it costs is stated rather than hidden. On a copy of a real
    42,538-term store the vocabulary table is created in about 0.1 ms once per
    connection, and a corrected query pays 20 to 30 ms for the scan of the
    21,241 terms near its length, the same order as the widened stage's own
    note. It is not a per-prompt cost: the strict pass answers almost every
    prompt, and this stage is reached only when the strict pass and the two
    fragment stages above it all came back empty.

14. **A query longer than the byte cap is refused, at one choke point, before it
    is tokenised.** The raw query string had no bound, so a pasted log was
    tokenised whole and the strict stage built one conjunction term per distinct
    word — the cost of the input, paid before any stage could answer. A query of
    more than 8192 bytes is refused with `query_too_long`, which names the size
    it saw and the cap. It is not `invalid_search`: an empty query and an
    over-long one are different mistakes with different remedies, and a caller
    reading only the code has to be able to tell them apart. The unit is bytes of
    the raw query, whitespace included. The cap is applied at one point in the
    store that both `mem_search` and `leteo search` pass through, so the two
    surfaces refuse identically; it lives in `MAX_QUERY_BYTES`.

    The number is measured rather than chosen. Through this crate's own store on
    a synthetic 4,000-memory corpus, `All` mode, mostly-distinct tokens, best of
    five runs:

    ```text
      query bytes   terms   strict pass
          257        31       3.8 ms
        1,031       119       7.0 ms
        8,195       882      18.5 ms
       32,778     3,304      61.5 ms
       65,542     6,530     132.5 ms
    ```

    The cost is close to linear in the terms, so the cap is what bounds it: at
    8 KiB the strict pass is under 20 ms on this corpus, and the same query in
    `Any` mode — bounded at thirty-two terms by §10 — is 3.2 ms at every size. A
    sentence or short paragraph is well under 1 KiB, so 8 KiB leaves an order of
    magnitude of margin above any legitimate question and refuses the pasted log
    that motivated the cap.

15. **When the words cannot answer, the search goes on to look by meaning.**
    A seventh stage, after the six lexical ones, finds a memory that shares no
    word with the question: a paraphrase, the same question asked in another
    language, a memory written in a language the question was not. It is
    in-process — a static embedding model read from a file, so no network at
    search time, no server and no shell-out, and nothing leaves the machine.

    **The model.** `sentence-transformers/static-similarity-mrl-multilingual-v1`
    (Apache-2.0), truncated to its first 256 dimensions, quantised to int8, with
    its WordPiece vocabulary pruned to the 49,203 pieces the thirteen interface
    languages use. It is 12.9 MB in three files — `model.safetensors`,
    `config.json`, and the tokenizer stored as 321 KB of deterministic gzip and
    decompressed when the model loads — run by `model2vec-rs` (pure Rust,
    `fancy-regex`, no `onig`). `tools/semantic/` rebuilds the files from the
    original and `checksums.json` records what each hashes to, stored and
    decompressed; the attribution is in `NOTICE`. A memory is embedded from its first 128 tokens, title first: that
    beat 64 and 256, and at 512 the semantic MRR of bodies fell from .57 to .35.
    It was chosen over Model2Vec models distilled from multilingual teachers,
    which come to 19.0 MB: on semantic-only MRR@10 over a copy of a real store it
    scored .62 on titles against .51 for the bge-m3 distillation and .38 for the
    e5-small one, and on the hard set it beat the bge-m3 one by .049
    [.035, .063], paired.

    **Where the model is, and how it arrives.** Nothing is embedded in the
    binary: a model compiled in ties the crate's size, and with it whether the
    crate can be published at all, to the model's, and the crate was 11 MB against
    crates.io's 10 MiB. Every install loads the model from a file and behaves the
    same at run time; only how the file arrives differs. It is looked for in one
    ordered list, `semantic::locations`, and nowhere else:

    1. the directory `LETEO_MODEL_DIR` names;
    2. `model/` beside the executable, the symlink-resolved directory first and
       then the one it was found in (a Homebrew `bin/` link points into the
       Cellar, which holds the rest);
    3. `../share/leteo/model/` from the executable;
    4. `model/` in the data directory.

    A release archive carries the model beside the executable and the Docker
    images put it under `share/`; the install scripts and the npm wrapper keep it
    where the binary looks. Every install that arrives without it -- `cargo install`, a build from source, a distro package, a
    manager added later -- is served by one path that knows nothing about which of
    them it is: `leteo model install` downloads the three files from the files
    committed under `assets/model/` at the tag of the binary's version
    (`raw.githubusercontent.com/<owner>/<repo>/v<version>/assets/model/`, with the
    owner and repository taken from the crate's own `repository` field, so no
    release step has to publish anything for it to work) into the data directory.
    `--url` or `LETEO_MODEL_URL` names another directory of the three files under
    their own names, and `--from <directory>` copies from a local copy and touches
    no network. `leteo setup` runs it when no verified model is found, after the
    agent is configured and never failing over it. The whole download has one
    deadline of two minutes, and each file is cut off at a known bound (the
    weights at 16 MiB, against 12,596,056 bytes). Nothing is written until every
    file has verified, and an installed model is replaced by renaming a finished
    directory over it.

    **The binary pins the model it accepts.** The SHA-256 of each file, as
    stored, is compiled into the binary (`semantic::MODEL_FILES`), and a test
    holds the list to `tools/semantic/checksums.json`, which the pipeline writes.
    `MODEL_ID` carries the start of the weights' hash, so the name stored beside
    every vector changes whenever the bytes do. A file that is missing or does not
    hash to its pin is never loaded: the stage is off and search is lexical only.
    `doctor` (`semantic_model`, [`store-and-schema.md`](store-and-schema.md) §4)
    says which condition holds -- verified and where, missing, or present and
    wrong -- and names the fix.

    **When it runs.** Only when the lexical stages answered nothing, or answered
    from `nearest`, the weakest of them:

    1. On an **empty** answer it returns the memories whose cosine with the
       question is at least `semantic::FLOOR`, 0.30. Below the floor the answer
       stays empty.
    2. On a **`nearest`** answer it merges the semantic list with the lexical
       one by reciprocal rank fusion, as the two full-text indexes are merged in
       §2, with no floor — the lexical stage was answering anyway — and only the
       best `semantic::MERGE_CAP`, 5, of the semantic list. The lexical answer
       already stands; everything the cosine brings past its best is padding. A
       question the store cannot answer reaches `nearest` through one word it
       happens to share, and without the cap that one word comes back as a full
       page of unrelated memories.
    3. An answer from any stronger stage is never touched, and does not so much
       as load the model. A search that answers is the search it was.

    It is also off in `mode: any`, which asked for a disjunction, and when a topic
    key answered. The floor is a cosine and not a rank because the stage has to
    be able to say nothing: without one it answers every empty question, the
    ones the store cannot answer included.

    **What it may return.** Exactly what every other stage may. The question
    "which memories can a search return at all" is one clause,
    `visible_observations` in `src/store/search.rs`, and the ranked stages, the
    title scan and this stage all read it: not deleted, not hidden by a judged
    verdict (§9), inside the type, project and scope asked about. Session
    summaries are left out of an ordinary answer, for §6's reason, but a query
    that names the type reads them, and they are given a vector where they are
    written like any other memory.

    **Where the vectors live.** In `observation_vectors`, one row per memory,
    made where the memory is written: a save, a revision, a consolidation, an
    import and an adoption each give the row they touched a vector and keep it,
    and a bounded background backfill fills what an existing store is missing.
    The search reads them and writes nothing ([`store-and-schema.md`](store-and-schema.md)
    §16), which is what takes the one-time cost off the search's path. Whether a
    vector is current is a function of the row — its content hash and title, and
    the model that made it — so every write path is covered without any of them
    remembering to say so. A memory with no current vector is simply absent from
    the answer; the backfill makes it, or `doctor --repair` on demand. A store
    that cannot be written still answers: it answers from the vectors already
    there, and nothing is made for the question.

    **How it says so.** A row it added carries `semantic: true`, and the answer
    carries one sentence saying such a row may contain none of the words asked
    for. On a `nearest` answer only the rows the stage added are marked, not the
    ones the words found. `leteo search` prints the same sentence on stderr.

    **The retry runs the stage as well.** The sentence that says "N elsewhere"
    on an empty answer is produced by asking the same question once more without
    the project narrowing, and that question goes through the same search. It
    is deliberate: the number is the number `all_projects` would return, which
    is what the sentence promises, and a retry that skipped the stage would
    report "nothing elsewhere" about a question the wider search answers. It
    costs what a search in the wider scope costs, and no vectors are made for it.

    **The setting.** `semantic_search` in `settings.json`, on unless it is
    `false`. Off, the search is the lexical one, no vector is made on any write
    path, and the backfill does nothing. A library caller that does not set
    `SearchOptions::semantic` gets the lexical search too; the two surfaces an
    agent or a person searches through read the setting.

    **What it was measured to buy**, against the same binary with the setting
    off, so the only difference is the stage:

    ```text
    engram-bench (117 queries)   lexical   with the stage
      ALL                          .835      .887
      spanish                      .516      .790
      multiword                    .923     1.000
      paraphrase                   .826      .841
      longnl                       .760      .775
      partial / short / typo       .923 / .938 / 1.000, unchanged
      empty answers                  9         3
    ```

    No kind falls. `floors.json` holds the new values and a ceiling of three
    empty answers.

    The hard set, `tools/semantic/hardset/` — 2,028 questions that do not use the
    target's words, **generated by an LLM and unchecked by native speakers**
    (the Basque and Galician translations especially) — is where the stage has
    something to do, and the evaluator reports it with a paired percentile
    bootstrap over questions, 4,000 resamples:

    ```text
                                  n   lexical  +stage   delta [95% CI]
      the hard set             2,028     .191    .291    +.101 [+.088, +.114]
      English paraphrase         120     .175    .263    +.089 [+.048, +.136]
      question in another lang.  960     .070    .190    +.121 [+.102, +.138]
      translated memories        948     .315    .397    +.082 [+.064, +.102]
    ```

    The English paraphrases are checked to share no content word with their
    target (`check_sets.py`). The gain is smaller than the .143 the same model
    showed in simulation, and the difference is the floor: the simulation
    answered every empty question and the product does not. `tools/retrieval
    --pipeline` asks the whole search over a copy of a real store: no set moves,
    to four decimals, because those questions are built from their targets'
    words and the lexical stages answer all of them. That is the property being
    checked — the stage does no harm where it has nothing to do — and not
    evidence that it helps.

    **What it does with a question it cannot answer.** `tools/engram-bench`'s
    corpus carries 24 questions about things no memory in it is about -- parental
    leave, travel expenses, the coffee machine -- and the harness reports, per
    engine, how many come back with results, how many of those carry no sentence
    saying the match is weak, and how big the replies are. With the model, 21 of
    the 24 come back with something, and none of them without a caveat: a reply
    that is not empty either reached the lexical stages through one shared word
    or carries the sentence above. What the cap moved is the size: the mean
    reply on those questions fell from 10,900 to 5,493 bytes when a `nearest`
    answer stopped merging the whole semantic list and kept only its best five.
    `floors.json` holds both as ceilings, 0 and 5,500.

    **What it costs**, measured on an Apple M-series machine:

    - **The binary** is 17,206,608 bytes on arm64 macOS and 21,052,600 on
      ubuntu-latest, which is the build CI checks; on macOS it was 15,730,544
      with the installer alone and 15,680,176 before either. The difference is
      the loader and its tokenizer, which the linker keeps only where something
      loads the model, and none of the model. The bound on it is in
      `tools/semantic/check_binary_size.py`, applied in the `search-quality` CI
      job, and is set from the Linux measurement. It bounds the code and is not
      tied to the model, which is a file beside the binary.
    - **The crate** is 1,076,718 bytes packaged, about 1 MB, and does not depend
      on the model's size, so a larger model can never again make it unpublishable.
      The model is 12.9 MB beside it.
    - **Memory** -- the one place it is stated. 15.7 MB resident for a search that
      does not reach the stage; about 103 MB for a process that has loaded the
      model, because the int8 table is expanded to f32; 121 MB at the peak of a
      backfill on a 4,000-memory store. What bounds that peak is the chunk:
      memories are embedded and kept 256 at a time, so the text and vectors held at
      once do not grow with the store (the ids of what is stale do, at about a
      hundred bytes each). The search pays none of this: it reads the vectors that
      are there and holds one question's. A 1,030-memory 120-query MCP run peaks at
      104 MB, which is the model's own footprint and not the vectors; a run that
      never reaches the stage stays at 19 MB.
    - **Time.** An ordinary search is unchanged: a strict answer on a
      4,150-memory real store took 6.2 ms before and 5.6 ms after. An empty question there goes
      from 48.7 ms (the lexical stages, which are most of it) to 78.5 ms, p90
      52.9 to 83.4; with the model read from a file it is 47.4 ms without the
      model to 81.4 with it (p90 51.5 to 102.4), the stage being 12 ms to verify
      and load the model and embed the question, 6 ms to find which vectors are
      current and 6 ms to scan 4,048 of them. Hashing the 13 MB of model files
      before loading them, which is what makes an unverified file never load, is a
      few of those milliseconds. Through `mem_search` over MCP on the hard-set stores
      the whole run's p50 is 1.8 against 1.9 ms and p90 3.3 against 3.6 ms,
      because almost no question reaches the stage.
    - **The first fallback search on a store with no vectors** no longer embeds
      anything: on a 54,050-memory copy it is 0.25 s and writes nothing, where
      making a vector for every memory in scope first cost 2.39 s and wrote 41,975
      of them. The same copy with its vectors already present answers the same
      question in 0.14 to 0.23 s, so a search that had a cache pays nothing new.
    - **The backfill** on a store gives a vector to every memory that lacks one,
      256 at a time and off the search path: 0.51 s for 4,048 memories on the
      real-store copy, and the store grows by 5.5 MB (1 KB a memory). The cost is
      linear in the store and paid once per memory, and the search that used to
      pay it for the memories in scope now pays none of it. `doctor --repair`
      runs the same pass to completion on demand.

    **What it does not do.** It does not fix Basque, which the model was not
    trained on: the 47% cross-lingual alignment of its UI strings, against
    89–97% in the other twelve, is why Basque questions improve least
    (`xl_eu` .000 to .037, against .08 to .17 for the others) and why the floor
    costs it most. 0.25 is the measured alternative to 0.30; it answers 42% of
    controls against 24% and keeps 67% of what can be rescued against 43%. It
    does not make silence reliable: of the 117 engram-bench questions asked in
    the project where their target does not exist, the stage turns 15 more into
    answers (54 to 69), and 7 more of the 160 questions about nothing in either
    project (114 to 121). The floor was calibrated on half the empty answers and
    reported on the other half, with 55 controls a half, which is about ±12
    points. It was not measured on real non-English questions put to a real
    store. And it reads the first 128 tokens, so a memory whose subject is
    stated late is found by its start.

16. **Every memory is also stemmed in the language it was written in, and the
    row says which.** `porter` is an English algorithm: it folds `memoria` and
    `memorias` by stripping a final `s`, and nothing else Spanish — `ejecuta` and
    `ejecutaron` stay two terms. Each memory therefore carries a second set of
    stems, made by the Snowball stemmer of the language in
    `Settings::language`, the language memories are written in,
    and a row in `observation_stems` records that language beside them. The
    language is taken when the row is written and kept until the memory's text
    (title, content or topic key) changes, which stems it again in the language
    of the connection that changed it; an edit that touches only `project`,
    `type` or `tool_name` keeps the language and the stems and refreshes the
    copied identifiers, so a project merge or a replicated project move after a
    change of setting does not turn a Spanish memory into an English one. `porter`
    stays on every row, because agents write English terms into memories in any
    language. Nothing is detected and no setting was added: a memory takes the
    setting as it stands when the row is written, so the choice is fixed per row
    and a store holding both languages is deterministic by construction. A
    setting that names no language (`auto`) or one that cannot be read indexes as
    English, which has no second stemmer and needs none.

    FTS5 takes its tokenizer from a fixed list, so a stemmer written in Rust
    cannot be named in `tokenize =`. The text is stemmed *before* it is indexed,
    by `leteo_stem`, a SQL function the connection registers
    (`src/stemming.rs`), and held in `observation_stems`, which `observations_stemmed`
    (`unicode61`, external content) indexes. The triggers that fill it are on
    `observations`, so every write path stems and none has to remember to: a
    save, a revision by topic key, an `update`, a project merge, a pulled
    replication write and the migration's own backfill. An import is the
    exception: it drops the triggers to build the indexes once at the end, and
    stems what it wrote with `restem_observations` before that rebuild, so a
    repair and an import share one stemming step. Rows with no stemmer for their
    language still get a row — language recorded, stems empty — so a stemmer
    added later can find the rows it owes.

    **At query time** the question is stemmed with `porter` (the existing index)
    and with the Snowball stemmer of every language *present in the rows*, not
    the one the setting names today: a person who switched from Spanish to
    English still holds Spanish memories. The Snowball lists are merged on a
    memory's best rank and fused in the strict stage as the third list; the
    widened and nearest stages take a memory's best rank over `porter` and
    Snowball, because they decide on bm25 against a floor. The prefix and
    substring stages and the vocabulary the typo stage corrects against do not
    read it: a fragment is not a word a stemmer has an opinion about.

    **Four surfaces are `porter` only, on purpose and unchanged:** the
    candidates `mem_save` offers for conflict detection
    (`src/store/relations.rs`), the per-prompt hint, the `type`/`project` list
    filter (`src/store/observations.rs`) and the session filter
    (`src/store/sessions.rs`). Each reads `observations_fts` directly, so on a
    Spanish store they miss an inflected match that `mem_search` finds. They ask
    a narrower question than a search, and putting a second index behind them
    would change what they were measured to do.

    **Which languages have a stemmer, and which do not.** `stemming::algorithm`
    is the one table. It carries eleven of the thirteen languages the setting
    offers, from two Snowball sources: Spanish, Portuguese, French, German,
    Italian, Romanian, Dutch and Swedish come from `rust-stemmers` 1.2.0, and
    Catalan, Basque and Polish come from `snowball_stemmers_rs`, because
    `rust-stemmers` 1.2.0, last released in 2019, does not carry them. The two
    sources are not interchangeable: `snowball_stemmers_rs` also carries the
    other eight, but it disagrees with `rust-stemmers` on words they already
    index — Dutch alone differs on 102,728 words of this repository's own
    vocabulary — so the eight keep the source their floors were measured
    against. English is `porter`'s, and one language the setting offers has no
    arm at all:

    - **Galician stays on `porter`.** Snowball has no Galician algorithm, and
      the languages close to it were measured against a Galician set rather than
      assumed. Portuguese, which the crate already carries and which Galician is
      usually grouped with, folds one of that set's eight queries, because it
      leaves the `-ción` family untouched and that family is most of Galician's
      technical vocabulary; Spanish folds six, but it is a different language
      and a borrow rather than a source. So no arm ships: the set is in
      `tools/engram-bench/inflection_sets.py`, it scores 0.000 on `main` and
      0.000 under `--porter-only`, and its floor is that 0.000.

    A row in a language with no arm here records its language and gets `porter`
    alone, and the rows recorded under a language are re-stemmed by the repair
    (`doctor --repair`) the day an arm for it exists, because the repair writes
    the stems of any row whose stems differ from what its recorded language
    makes of its text.

    **The language of a row is fixed, and a repair does not change it.** A store
    whose setting is `auto` records its memories as English, and naming a language
    later does not re-stem them: a row stemmed under `en` cannot be told from a
    memory written in English by someone who named English, and handing every
    such row to a new setting would stem real English text in the wrong
    language. What a later write does is stem *its own* row in the new language.
    A store that wants its old memories under a new language has no way to ask
    for it yet.

    **What it costs, and what it did not buy.** A larger index — 782 KB to 954 KB
    on the 178-memory benchmark store — and some cross-language false positives:
    a Spanish-language store stems its English memories with the Spanish
    algorithm too, and a query is stemmed the same way. A connection that has not
    registered the functions cannot write a memory, and fails saying so; a
    foreign tool writing to the file directly is such a connection. Migration 21
    stems the rows that exist in the language the setting names when it runs,
    because nothing recorded what their writer used.

    **Each language that ships a stemmer has a set that can fail.** The
    ratchet's `spanish` kind could not show the stemmer working (below), so
    every language with an arm in `stemming::algorithm` has eight questions in
    `tools/engram-bench/inflection_sets.py`, each reaching its memory only
    through another inflection of a word the memory holds — `reindexar` for
    `reindexación`, `Verarbeitungen` for `verarbeiten` — at least three edits
    from it, which is past the typo stage's budget. The set is scored on the
    strict pass alone: an answer counts only if no result is marked `partial` or
    `semantic` and the hint does not say terms were corrected, so a prefix,
    substring, typo, widened, nearest or semantic rescue scores as a miss. The
    runner refuses to run if a deliberately mangled word is ever answered without
    saying so, because then it could not tell the stages apart. One store per
    language, with that language named in its settings; `inflection_check.py`
    verifies the edit distances and, where the language has an arm, that its
    stemmer folds each query word onto its memory. A stemmer ships only if its
    set separates the stemming store from the same store told it writes English:
    every language with an arm scores MRR 1.000 with its stemmer and 0.000
    without it, and a language with no arm has only its distances checked and
    scores 0.000. A question is worth keeping only if
    `porter` cannot answer it: French's first draft had four that share an
    English suffix, `-ation` and `-er`, and `porter` answered them. The floors are in `floors.json` under `inflection`.

    Measured on `tools/engram-bench` with the semantic stage on, as CI runs it,
    the `spanish` kind went 0.790 to 0.969 and overall 0.887 to 0.918 — and
    **almost all of that is the corpus, not the stemmer**. Eight of the sixteen
    Spanish questions asked in Spanish for memories written in English, which no
    stemmer can bridge; the corpus now holds Spanish-written counterparts of
    those eight. The unmodified binary on the extended corpus scores 0.969 on
    `spanish` as well, so Snowball moved that kind by nothing: every question
    that missed, missed on a word the memory does not contain, not on an
    inflection. What the stemmer did move is `paraphrase` (0.836 to 0.856) and
    `longnl` (0.775 to 0.792), English kinds, in a store told it writes Spanish.
    The cases where it decides — a strict question one inflection away — are
    held by `src/store/tests/stems.rs`, each written so that `porter` alone
    cannot pass it. `tools/retrieval` reads `observations_fts` through the shipped
    statement, which this change does not touch, and measured the same to four
    decimals before and after on the benchmark's own store.

## Invariants

- The index is kept level with its table by triggers and by nothing else.
  Losing one is silent — an edited row leaves both row counts equal — so
  `doctor` calls the roll of triggers by name. See
  [`store-and-schema.md`](store-and-schema.md) §6.
- A migration that rewrites a column a full-text index carries rebuilds those
  indexes. Not because FTS5 cannot see a plain `UPDATE` — in this schema
  `obs_fts_update` and `obs_exact_update` are bare `AFTER UPDATE ON observations`
  with no `UPDATE OF` list, so they fire on any column — but because a migration
  that writes around the triggers leaves them stale. One that touches no
  full-text column needs neither: see [`store-and-schema.md`](store-and-schema.md)
  for the same invariant.
- The stems are kept level with the memories by triggers and by nothing else,
  exactly as the other indexes are, and they need the functions the connection
  registers. A write path that bypasses the triggers has to call the repair
  (`restem_observations`) before it commits; the import does.
- Ranking transfers between SQLite builds; timing does not, and neither does
  query *construction*. A measurement of search quality made anywhere other than
  through this binary's own query builder is a measurement of something else.
- The semantic stage never returns what another stage may not, and never runs
  when a stronger stage answered. The first is one shared clause (§15), the
  second is held by a test that asks a question each stronger stage answers and
  finds no vector written.
- Search quality has a floor that CI enforces. `tools/engram-bench/ratchet.py`
  saves a fixed synthetic corpus into a fresh store through `mem_save`, asks every
  query it defines, in several kinds, through `mem_search`, and fails when any
  kind's mean reciprocal rank, or the overall one, or a language's strict-pass
  rank on its inflection set, is below its entry in
  `tools/engram-bench/floors.json`. The same file holds a byte ceiling for
  twenty-result `mem_search` replies and for `mem_context` with its defaults,
  because a change that keeps ranking and doubles what every call sends is a
  regression too. A run that evaluated fewer queries than the corpus defines, or
  could not start the binary, fails rather than passes. Time is printed and never
  gated, for the reason in the previous item. A change that moves a number
  deliberately edits `floors.json` in the same commit; see `tools/README.md`.

## Where it lives

- `src/store/search.rs` — the stages, the fusion, the floors
- `src/memory/normalize.rs` — `fts_query`, `topic_key`, and the narrowing folds
- `src/store/schema.rs` — the indexes and the triggers that feed them
- `src/stemming.rs` — the language table, the stemmer, and the SQL functions
- `migrations/0021_observation_stems.sql` — the stems, their index and triggers;
  `migrations/0022_stems_keep_their_language.sql` — the trigger split that keeps
  a row's language through an identifier edit
- `tools/engram-bench/inflection_sets.py`, `inflection.py`, `inflection_check.py` —
  the per-language strict sets, their runner, and the checker of their edit
  distances
- `src/store/tests/search.rs` — the stage-by-stage tests
- `src/semantic/` (the model, where it is, its pins, `install`), `src/store/semantic_stage.rs` — the model, the stage, and
  the fusion; `src/store/tests/semantic.rs` holds them
- `assets/model/`, `tools/semantic/` — the weights and the pipeline that makes
  them; `tools/semantic/hardset/` — the hard set and its evaluator
- `tools/retrieval/` — the self-retrieval harness, and the reranked variant of
  the ranking statement it measures; `--pipeline` asks the whole search with and
  without the semantic stage
- `tools/engram-bench/ratchet.py`, `floors.json` — the quality and reply-size
  floors (the ratchet names the memory language in the store's settings), the
  per-language strict floors, and the ceiling on empty answers, run by the `search-quality` job in
  `.github/workflows/ci.yml`, which also runs the hard-set evaluator, the
  binary-size check and `tools/semantic/check_install.sh`, which installs a
  release-shaped archive and checks the binary finds its model and an uninstall
  takes it away
- `.github/workflows/release.yml`, `scripts/install.*`, `npm/bin/leteo.js` — how
  the model reaches an install: packed beside the binary in every archive, and
  put where the binary looks when an installer or the npm wrapper unpacks: `../share/leteo/model` for `install.sh`, `model/` beside the executable for `install.ps1` and npm. Nothing is
  published for `leteo model install`, which reads the files committed at the tag

## Related

- [`memory-model.md`](memory-model.md) — the fields being matched
- [`store-and-schema.md`](store-and-schema.md) — the indexes, triggers, and `doctor`; §17 is the stems table
- [`mcp-tools.md`](mcp-tools.md) — `mem_search`, and the hints §4 describes
- [`hooks.md`](hooks.md) — the prompt nudge, which is a search nobody asked for
