# Store and schema

## Purpose

The database underneath every surface: the shape it converges on, how it gets
there from any provenance, and how it says when something has gone wrong.

## Behaviour

1. **One SQLite file, WAL, opened per process.** No connection pool across
   processes and no cache between them: each surface reads the database when it
   answers.

2. **Any Leteo database converges on the baseline by inspection.** Migration 1
   is in three parts — tables, data normalisation, then the triggers, which
   reference columns a legacy database only gains while being adopted.

   Engram's own database is refused rather than converged, and named: it has
   `user_prompts` where Leteo has `prompts`, and the answer says so and points
   at `leteo import --from-engram`, which snapshots the source and writes into a
   Leteo store without touching theirs. Told apart by shape and not by version,
   because `user_version = 1` means two things — Engram stamps its own schema
   with it, and Leteo stamps a database it has just converged. Pointed at a real
   Engram backup, every command used to answer `no such table: prompts`.

3. **One migration, numbered 1, and it is the baseline.** The schema is
   `include_str!`-ed at build time so the binary needs no files beside it. A
   *released* migration is never edited: a database that already ran it will not
   run it again, so editing one silently splits new databases from old.
   `SCHEMA_VERSION` is what this build understands; a database stamped higher is
   refused at `open` rather than opened hopefully.

   Eleven accumulated before the first release — ten folded into one numbered 16,
   then one more numbered 17 — and they are the baseline now, with the numbering
   started again at 1. That is the rule applying rather than an exception to it:
   what it protects is a released migration, and nothing had been released, so
   there was no population to split. Verified the way the earlier collapse was:
   a database built by the numbered path and one created from nothing were
   compared object by object — seventy-six each, none missing on either side, no
   difference in any definition.

   What it costs is stated rather than discovered later. A store carried through
   development is stamped somewhere in 2..=17 and is refused, because from here
   on a number above `SCHEMA_VERSION` means a newer build wrote the file and
   guessing which of the two it is would be worse than either. The one store in
   the world in that position is re-stamped by hand; code that recognised the
   old numbering would outlive its reason.

   And the renumbering brought back an ambiguity that numbering above 1 had
   retired: **Engram stamps `user_version = 1` too**. The fast path that skips
   migration when a database is already at this version matched another
   program's file exactly and opened it as Leteo's own — `migrate` tells the two
   apart by shape and never got the chance. The fast path checks the shape as
   well now, which is one lookup in `sqlite_master`. Found by the guard that
   exists for it, on the first run after the renumbering.

4. **`doctor` reports, and `doctor --repair` fixes what it can — and every check
   it can fix says so.** A failing check that `--repair` undoes ends its sentence
   naming the flag; one it cannot undo says something else. Current checks:
   `sqlite_integrity`, `foreign_keys`, three full-text `*_integrity` checks,
   three `*_sync` row-count checks, `observation_hash_sync`,
   `observation_type_searchable`, `full_text_triggers`, `topic_key_uniqueness`,
   `settings_readable`, `journal_mode`, `busy_timeout`.
   Every code is listed
   once in `DoctorCheck::CODES`, and tests hold the list to the checks that run.

5. **A hash that has stopped describing its memory is found and put back.** The
   body is the truth and the hash is derived from it, so taking it again is the
   whole repair. A real store of 3,940 memories held three such rows — memories
   nothing could ever be deduplicated against, silently and for good.

   Beside it, `observation_type_searchable` reports a memory filed under a word
   no filter can ask for. The category is a search filter; the save door folds
   the close synonyms and keeps anything else verbatim, which is deliberate, and
   says so at the moment it happens — but nothing ever said it about the
   memories already in, so a store that collected them before that hint existed
   had no way to find out. A real store of 4,121 held thirty-eight, under five
   words. The words are what somebody acts on, so the check names them with
   their counts, commonest first, capped at eight with the rest counted, and
   asks for the closest of the eight kinds by reading `KINDS` rather than
   repeating it. No `--repair`: which of the eight a memory belongs under is a
   question about what it says, and Leteo does not read them.

6. **A missing full-text trigger is named and restored.** The triggers are the
   entire mechanism keeping an index level with its table; nothing else writes
   to one. A store that loses one goes on answering searches with yesterday's
   words, and the row-count checks cannot see it because an *edited* row leaves
   both counts equal. `--repair` restores the missing statements by reading them
   out of the migrations that define them — read rather than copied, so the SQL
   cannot drift — and rebuilds the indexes afterwards, because restoring a
   trigger stops the drift growing but does not undo it.

7. **A check that could not run says so.** "Failed its integrity check" and
   "could not be checked" are different sentences, and reporting the first for
   the second sends somebody to rebuild an index that was never broken.

8. **`busy_timeout` is reported as configured, not as hoped.** On Windows the
   handler overshoots its nominal wait by 10–13%: SQLite's busy handler sleeps
   in a ladder topping out at 100 ms and the platform rounds every step up to
   15.6 ms. Anything that budgets against a wait accounts for the overshoot —
   see [`hooks.md`](hooks.md) §2.

9. **No caller's text is ever pasted into SQL.** Sixty statements are built
   with `format!`, and every interpolation is a constant of this crate — column
   lists, table and index names, a bm25 weighting, a placeholder run. Values
   travel as bound parameters.

   The one place a name comes from outside is adoption, which reads a database
   Leteo did not write: the column list it copies is the *intersection* of both
   schemas, so a name survives only if Leteo has it too. A guard hands adoption
   a column called `x); DROP TABLE observations; --` and requires it not to come
   back.

10. **A column nothing writes says so, and here they are.** Six of them, found
   by counting non-null values on a real store and then grepping for a writer.

   `observations.embedding`, `embedding_model` and `embedding_created_at` are
   inherited from the upstream schema and argued for in the baseline migration
   itself: Leteo embeds nothing, retrieval is FTS5 with weighted bm25 and the
   semantic half is a model judging a pair, so they stay because an adopted
   database has them.

   `observations.expires_at` and `memory_relations.superseded_at` /
   `superseded_by_relation_id` are the same kind of thing with nothing said
   about them, which is why they are written down here. Nothing expires a
   memory; supersession is a relation whose verb is `supersedes` and whose
   `judgment_status` is `judged`, which is a different question from a relation
   being replaced by a later one. A reader of the schema would reasonably assume
   both features exist.

   Not deleted. Dropping a column is a migration against every database that
   already holds one, and the cost of carrying six nulls is a row byte apiece —
   but a column that promises a feature has to be either the feature or a
   sentence, and until now three of the six were neither.

11. **A topic key that names two live memories is reported.** One live memory
   per key, per project, per scope — the revision lookup finds it by exactly
   that triple ([`memory-model.md`](memory-model.md) §10) — and merging two
   projects is the one thing that can break it. The merge says how many it left
   sharing a name, and that report is a number in one reply; the state it
   describes is permanent. Nothing mentioned it again and `doctor` called the
   store healthy, while the next save under that key revised whichever row the
   lookup reached first and left the other unreachable by its own key for good.

   No `--repair`: which of the two keeps the key is a question about what they
   say, and Leteo does not read them, so the check names the remedy instead.
   `journal_mode` and `busy_timeout` aside, it is the only check whose red a
   deliberate operation can produce — the fuzz over random sequences allows it
   exactly as far as the merges owned up to, and requires every other check
   green.

12. **An answer in the settings file that Leteo reads past is named.** Each
   field falls back to its own default when it cannot be read, which is what
   stops one typo taking the rest of the file with it, and a file that is not
   JSON at all is survived rather than refused — hooks read it on every event.
   Both are right and both are silent, so `doctor` does the same reading once
   more out loud: `context_size` set to `slimm` is named with the value that was
   in it, and so is a key that is not one of the five, which is the same typo
   one letter earlier. What is applied is unchanged; what is ignored now has
   somewhere it is said.

13. **A question a session opening asks every time is read through an index.**
   The opening block asks which sessions were touched last, and that means
   counting a project's memories per session and taking each one's newest date.
   `idx_obs_session` finds a session's rows and stops there, so the group read
   the table for every one of them — bodies included, to use a count and a date.
   Measured on a real store with nothing else changed: 3.36 ms of a 7.53 ms
   opening block on the largest project, 0.64 ms after; 2.13 ms against 0.16 on
   the next one down. Migration 16 adds the covering index, and the guard
   asserts the plan rather than the time, because timing does not transfer
   between SQLite builds and a plan that stops reading the table does.

   The pinned lookup was the exception, and it hid behind the word `SEARCH`.
   Every session opening and every `mem_context` asks a project for its pinned
   memories, and the plan said `SEARCH observations USING idx_obs_project_order`
   — which is `(project, datetime(created_at) DESC, id DESC)`, so the search
   walks every memory the project has, in date order, testing `pinned = 1` on
   each. At ten times a real store, 3,370 of 41,700 in that project, 5.57 ms;
   flat in whatever limit the caller asked for, because the limit is applied
   after. Migration 17 adds a partial index holding the pinned rows and nothing
   else — one entry on the store it was measured against — and the query drops
   to 0.00 ms. Through the binary: `leteo context` 19% faster, `--limit 80` 22%,
   the answer identical byte for byte, and `leteo recent`, which asks for no
   pins, unmoved.

   Three wrong turns are worth as much as the fix. An index was first built on
   `ifnull(project, '')`, which is not what `Narrowing::equals` writes, so it
   served a query nothing issues and changed nothing — and it was nearly
   committed on the strength of a comparison between two stores that both
   already had it. A rewrite of the recent-sessions group-by measured *slower*
   than what it replaced. And a timing table of the whole surface at scale was
   measuring the error path, because the store it ran against was stamped a
   version the binary refuses. The plan transfers between builds; a time
   measured outside this binary's own connection does not, and neither does one
   measured against a store that never answered.

   The guard reads the plan, and reads the schema for the partiality the plan
   cannot show: an index over the same columns without the `WHERE` is a second
   copy of `idx_obs_project_order` under a new name, walks exactly as it did,
   and is indistinguishable in a plan. And it explains the statement the code
   prepares rather than a copy of it, which is the mistake above written down as
   a rule.

   The rest of the surface was swept the same way and is clean, which is worth
   writing down because the first sweep was not. Capturing every statement the
   hot paths prepare — an opening block, a prompt hint, a search, the reread
   count, the project list — and explaining each, 19 distinct queries produce
   one plan with a temporary B-tree and none with an unindexed scan. The first
   pass looked for `SCAN` alone and called that clean; the session-list query
   was a `SEARCH` with a temporary B-tree over the group, which is how it
   survived a sweep that found nothing.

   The one that remained was `mem_stats`, whose project list was 7.7 ms of the
   9.4 that tool costs — the three counts beside it are 0.05 ms together. It was
   left alone once, on the grounds that fixing it wanted an index every store
   would carry forever and `mem_stats` is an admin tool nobody waits on. That
   reasoning had the wrong shape: the index it needed was already there.
   Wrapping the ordering in `datetime()` alone, which is what was tried, changes
   neither plan nor time, because the query still walks every live row through
   `idx_obs_project` — an index holding the project and nothing else — to reach
   `deleted_at` and `created_at`.

   Asked per project instead, the distinct names come out of that index without
   touching the table and each one's newest memory is a single seek into
   `idx_obs_project_order`, which is `(project, datetime(created_at) DESC, id
   DESC)` and already has that row first. Seventeen seeks against four thousand
   lookups: 0.02 ms in SQLite, and `mem_stats` measured over its own protocol
   goes from 9.4 ms to 0.3, with the same seventeen names in the same order and
   the same 793 bytes. No new index.

   The guard reads the plan, because a result-based test cannot tell the two
   shapes apart — which is exactly how this survived the sweep that found it and
   the one that measured it. The query lives in one constant so the guard can
   explain the statement that runs rather than a copy of it.

## Invariants

- Every full-text index has its triggers, and `FULL_TEXT_INDEXES` /
  `FULL_TEXT_TRIGGERS` are the single roll calls that `doctor`, the rebuild, and
  the restore all read.
- A migration that rewrites `observations` rebuilds the indexes; an
  external-content FTS5 table does not notice a plain `UPDATE`.
- A doctor message is one sentence with no source indentation in it. A guard
  test enforces this, after five separate occurrences of a formatted Rust string
  carrying its own leading whitespace into a user-facing line.
- **No test opens the real store.** Unit and integration tests run against a
  temporary database they create. `tests/repository_guards.rs` walks `src/`
  and `tests/` and fails on any absolute `.db` path, `home_dir()` join, or data
  directory near a call that opens a store.

## Where it lives

- `src/store/schema.rs` — the baseline, the migration list, the roll calls
- `src/store/diagnostics.rs` — every check, and the two repairs
- `migrations/*.sql` — the SQL, owned here and read from here
- `src/store/tests/schema.rs`, `src/store/tests/diagnostics.rs`

## Related

- [`search.md`](search.md) — what the indexes and triggers are for
- [`memory-model.md`](memory-model.md) — the columns these tables hold
- [`cli.md`](cli.md) — `leteo doctor` and `--repair`
- [`replication.md`](replication.md) — the second writer these locks contend with
