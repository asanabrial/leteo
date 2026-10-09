# Memory model

## Purpose

What a memory *is*, what fields it carries, and the rules that hold whatever
touches it. Every other spec in this folder either writes one of these or reads
one back, so this is the file to change first when the shape of a memory
changes.

## Behaviour

1. **A memory is an observation.** It has a title, a body, a type, a project, a
   scope, an optional topic key, timestamps, and counters for how often it has
   been revised and how often the same body has been saved again.

2. **Eight types, and one the store writes itself.** An agent files a memory as
   `bugfix`, `decision`, `policy`, `architecture`, `discovery`, `pattern`,
   `config`, or `preference`. `session_summary` is the ninth and is written only
   by Leteo when a session ends. Anything outside those nine folds onto
   `discovery` on the way in, so a type filter can always reach it; a store
   written before that fold still holds the old words, and `doctor` reports them
   as a warning until `--repair` folds them.

3. **Every type folds onto one of the eight.** `bug`, `design`, `learning`, and
   `setup` fold onto their one obvious target on the way in *and* on the way
   out: a memory saved as `bug` is stored as `bugfix`, and a search narrowed to
   `bug` finds it. A word the vocabulary does not name — `implementation`,
   `feature`, `optimization` — folds onto `discovery`, the bucket the rest of
   the unclassifiable lands in. Folding at one end only is worse than not
   folding, because the caller is told the memory is not there.

4. **Three types go stale, and say when.** A `decision` asks to be reread after
   six calendar months, a `policy` after twelve, a `preference` after three.
   Every other type is as true in a year as it is today and carries no review
   date. Calendar months, not multiples of thirty days — that is what the rule
   says in words. A calendar month that lands on a day the target month does not
   have is **clamped to that month's last day**, never rolled into the next one:
   a decision written on 31 August is due on the last day of February — the
   28th, or the 29th in a leap year — rather than rolling into March, which is
   where SQLite's `datetime(x, '+6 months')` puts it (the 3rd, or the 2nd when
   February has 29 days). That
   is `chrono::checked_add_months`, and it is stated here because the other
   plausible answer is what SQLite's `datetime(x, '+6 months')` does, which is
   how the baseline migration and this rule came to disagree on about seven days
   a year until migration 18 repaired it.

   Counted from the memory's own date, not from when a store heard about it: the
   clock is set in one place, and every path through it agrees, so a memory
   replicated five months late is due one month from now on both machines rather
   than six months from now on one of them.

   "One place" was written here while there were two. `reschedule_review` read
   the row's `created_at`, and the INSERT in `add_observation` read
   `Utc::now()` a few microseconds after SQLite had stamped `created_at` inside
   the same statement — allowed by a note saying that for a local save the two
   are the same thing, "`created_at` is a moment old". A moment is not nothing:
   a save that crossed a second boundary between them got a date one second
   past the one any other machine computes from the same memory, and the guard
   that compares two stores field by field failed on it about twice in
   twenty-five runs of the suite, oftener under load, because load is what
   widens a window measured in microseconds.

   The insert now goes through the same function, and what holds it is a
   structural guard rather than a value one: with two clocks the dates agree
   except at a boundary, so a test comparing them passes almost always with the
   defect in place. `only_marking_something_reviewed_reads_the_clock_for_a_review_date`
   counts instead how many places compute a review date from the clock. There
   is exactly one, and it is `mark_reviewed`, which means "six months from
   today" and is a different rule wearing similar words.

5. **Normalisation belongs to the store, not to a caller.** Project names,
   scopes, types, topic keys, titles, bodies, session summaries, and judgment
   text each have exactly one normaliser, and every write door applies it: the
   MCP tools, the CLI, the hooks, and the replicated paths in `store/wire.rs`.
   A rule applied on one path is a rule that does not exist.

6. **`[REDACTED]` is a promise.** Text wrapped in the private marker is written
   by the caller and never stored — in a title, a body, a prompt, a session
   summary, or the reason and evidence attached to a judgment. Guarded by
   behaviour rather than by a list of doors: the same secret is pushed through
   every text field of every write tool and then looked for in every text column
   of every table, including the mutation journal a replica would replay. The
   promise has been broken three times, in three different places, and the third
   was under that guard: it drove `mem_compare` with `reason` and `evidence`,
   which are `mem_judge`'s parameter names one tool over, so serde dropped both
   and the door was called with nothing to redact. It then judged the very pair
   it had just compared, and the redacted verdict wrote over the unredacted
   reasoning before the sweep looked. Two ways of passing on nothing, in one
   test. Every parameter type carries `deny_unknown_fields` now, so a name this
   surface does not have is refused with the names it does.

7. **A body is deduplicated by a hash of its normalised text.** Saving a body
   that matches an existing one bumps that row's duplicate counter instead of
   writing a second row. The hash is derived data: if it stops describing its
   own body, that memory can never be deduplicated against, and `doctor` says
   so — see [`store-and-schema.md`](store-and-schema.md) §5.

8. **Deletion is soft, and visible.** A deleted memory keeps its row with
   `deleted_at` set. Search, the opening context, and the timeline all exclude
   it; `mem_get_observation` still hands it over, reporting `state: "deleted"`,
   because an id in an agent's hand usually came from an older context and
   "that was deleted" is more useful than an error.

   Nothing brings a deleted memory back. Both lookups a save does — the hash
   and the topic key — filter `deleted_at IS NULL`, so neither can see the
   deleted row, and saving the same title, body and key again leaves two rows:
   one dead under the old id and one live under a new one. That is the
   behaviour, and it has had a test since before this was written. The refusal
   every other door returns promised the opposite — "saving it again is what
   brings it back" — so the store asserted one thing in its tests and told
   agents another, which is reporting the nearest hopeful state rather than
   what happens. The sentence says what happens now, and the test that checks
   it drives a save rather than reading the words.

9. **Pinning is local.** A pinned memory is shown first by *this* store. It
   travels in an export, because an export is this store written down, and it
   does not travel over replication, because one machine's shelf should not
   rearrange everybody else's.

10. **A topic key is a family and a segment.** `architecture/wizard-split`:
    lowercased, whitespace folded to hyphens, and idempotent — handing the
    normaliser its own answer returns that answer unchanged. It holds one live
    memory per project and scope, because revising one finds it by exactly that
    triple; saving cannot produce two. Merging two projects can, since each may
    have had its own memory under one key, and the merge reports how many it
    left sharing a name rather than choosing which one to keep.

    A merge says the three things it can change that nobody asked for: how many
    topic keys now share a name, whether replication had to follow the memories
    into the canonical project, and whether that project existed at all before
    the merge. The last is what tells a rename from a typo. Merging into a name
    the store has never held is the only way to rename a project, so it is
    allowed — and it is also what `to: "ledgerlya"` looks like: a whole project
    walks into the misspelling, the reply reports success, and the memories are
    findable only under the mistake. Every other write refuses a project nobody
    invented, and `project_exists` is the check that does it; this path never
    asked. It asks now, before anything moves, and says `canonical_created`
    when it moved something into a name that held nothing.

11. **Scope is a label, and today it is only a label.** A memory is `project`,
    `personal`, or `global` — one list in `normalize::SCOPES`, which every
    surface that names them reads, because four tool parameters and the skill
    said "project or personal" while this door took the third. Anything else is
    read as `project`. It partitions
    deduplication and topic-key revision alongside the project name, and a
    caller may filter a read by it. It does **not** change where a memory is
    filed or which reads find it: a `personal` memory still belongs to the
    project the directory resolved to, and a read narrowed to another project
    will not return it.

    A scope outside the three is *replaced*, and the reply says so. A type
    outside the nine is replaced too, but silently and onto `discovery` — a
    documented kind a filter reaches — so the scope is the one that needs a
    sentence. Here the caller's own value is discarded, and a read narrowed to
    the scope they asked for will never return the memory they believe they
    filed there. Replication and export ignore scope entirely. Whether
    that is right is
    [an open proposal](../changes/a-personal-memory-that-follows-you.md), which
    has the measurement that rules out the obvious version of the fix.

12. **A verdict is about two memories, so both have to be there, in one
    project.** Judging refuses a memory this store does not hold, on every door:
    the replicated one already deferred a relation whose ends had not arrived,
    and the two local ones asked nothing at all. A hard deletion marks what was
    said about the memory as `orphaned`, and judging cannot bring that back to
    life — a soft deletion leaves a row, so a judgment about it is still about
    something the store holds.

    **And a memory that changes project retires the proposals it strands.**
    Judging also refuses two ends in different projects, so moving one end out
    leaves a pending pair no call can ever settle — measured: two memories in
    `leteo`, a pair proposed, one moved, and the judgment comes back "a relation
    joins two memories of one project, and these are in leteo and otro". Nothing
    marked it, so it stayed `pending` and was counted in every queue that counts
    pending rows, for as long as the store existed. `mem_update` now marks those
    `orphaned`, which is the mark a hard deletion already used, because the
    state is the same one — a relation nothing can judge any more. One word for
    it keeps every `!= 'orphaned'` in the crate correct, the replication export
    included; a second word would be four more places to forget, which is why
    the deletion case was pulled into one function to begin with.

    Only the **pending** ones. A judged verdict survives the move, because
    `caveats_for` does not narrow by project: a `supersedes` recorded before it
    still warns the memory it overturned, on all six surfaces that show one, and
    tidying a proposal away must not take a real warning with it.

13. **A judged `supersedes` marks its target out of date, and hides it from every
    listing.** The relation is what carries the fact: the replacement names the
    memory it replaces, so a merge is traceable and reversible. A memory a
    judged `supersedes` points at is left out of every door that lists live
    memories — the search stages, the prompt hint, the pinned and recent
    context, the raw recent listing the CLI, the Obsidian view and the conflict
    scan read, the session summaries and the two pages — by one rule,
    `not_superseded`, interpolated into each query rather than restated, so a
    memory one surface has hidden cannot be handed over by the surface beside
    it. A *pending* relation hides nothing: only a verdict does, and a pair still
    waiting is a proposal ([`mcp-tools.md`](mcp-tools.md) §7). Reversing the verdict —
    `mem_judge` changing the verb or removing the row — restores the memory to
    every list, because nothing was deleted and no column was set.
    `mem_get_observation` still hands a superseded memory over, with the caveat
    attached, for the reason §8 gives about a deleted one: an id in an agent's
    hand usually came from an older context, and "this was replaced by #id" is
    more useful than an error. See [`mcp-tools.md`](mcp-tools.md) §19 and
    [`search.md`](search.md) §9.

14. **A content-changing write keeps the title and body it replaced.** Saving
    under a topic key that names an existing memory, or revising one with
    `mem_update`, overwrites its text in place — and used to lose it, leaving
    only `revision_count` as a mark that anything had happened. The replaced
    title and body are now written to `observation_versions` inside the same
    transaction as the write, keyed by the observation's `sync_id` and the
    revision number being superseded, so a read hands the previous text back
    byte for byte. `mem_get_observation` opens it with `include_history: true`
    ([`mcp-tools.md`](mcp-tools.md) §20); the live memory is unchanged in every
    other way.

    **A `mem_update` may write the whole body or edit one span of it.** With
    `find` and `replace` the body is read and edited inside the same transaction
    as the write, so the span is counted against what the row holds and the
    previous text is snapshotted exactly as a whole-body write snapshots it.
    The edit is refused unless it names one span ([`mcp-tools.md`](mcp-tools.md)
    §22); the title is never touched by it, because a title is written rather
    than edited.

    **Only a change to the title or the body counts.** A write that moves the
    project, the type, the scope or the topic key replaces no text and keeps no
    version, nor does a re-save of the same words under one key or a
    deduplicated save: none of those is a previous version anybody lost.
    Restoring is an ordinary `mem_update` with the old text — there is no
    separate undo, and nothing marks the live memory as unrevised.

    **Retention is the newest twenty revisions of one observation.** The bound
    is `store::OBSERVATION_VERSION_RETENTION`, applied on every insert by the
    one function both write paths call, so the number here and the number
    applied are the same, and the newest revisions are the ones that survive.

## Invariants

- The list of types exists once, in `rules::KINDS`, the review windows once in
  `rules::REVIEW_WINDOWS`, and the relation verbs once in
  `rules::RELATION_VERBS`. A test walks those lists rather than a copy of
  them: a hand-written third copy is what once let `policy` keep a window
  nothing could ever fire.
- **A released migration is the one thing allowed to freeze a copy of those
  numbers**, and it is written down here because an unexplained second copy is
  indistinguishable from the defect above. There are two, both licensed by the
  same rule: migration 18 carries its own
  `[("decision", 6), ("policy", 12), ("preference", 3)]`, and the baseline's
  three SQL literals (`'+6 months'`, `'+12 months'`, `'+3 months'` in
  `0001_baseline_after_the_tables.sql`) are the windows it actually wrote.
  Each is append-only and must give every database the same answer whenever it
  happens to run; reading the live list would make one migration produce two
  results either side of a window change, which is the split append-only exists
  to prevent. A migration freezes what it needs; nothing else may. The guard
  that counts the baseline's three literals is what keeps that number honest.
- **The version-history retention bound is one constant and one function.**
  `store::OBSERVATION_VERSION_RETENTION` is applied by
  `store::observations::snapshot_observation_version_tx` on the local write
  path and the replicated one alike, and §14 names the same number. A second
  copy — a `LIMIT` written into a query, or a bound on one path only — is the
  defect the single function exists to prevent.
- A title is one line, and no longer than a body. Both doors fold and bound it
  through `normalize::title`: saving folded and did not bound, updating did
  neither, so 200 KB went in and came back out of `mem_get_observation` from the
  full-text column weighted highest of the six. The bound is the body's rather
  than the display's, and the measurement says why: on a real store of 4,013 the
  longest title is 195 characters with 67 past 140, so cutting to what a title
  is *shown* at would take the end off titles somebody wrote on purpose.
- A title is shown at `normalize::TITLE_CHARS`, which every surface that renders
  one reads, and so does the passive capture that writes one. The capture cut at
  60 — below the *median* — and it was cutting what it had just been handed, so
  two in five learnings arrived with the point taken off and were then shown
  under a bound that would have fitted them whole. 140 was chosen as the p99 of
  that store's titles and is now a little under it: 147 today, which is what
  three thousand more memories did to the distribution.
- Every normaliser is idempotent. A property test hands each one its own output
  and requires it back unchanged.
- Any field a surface serialises is in `memory/model.rs`. No surface invents one.
- These hold in any order the operations can come in, not only the orders
  somebody wrote a test for: four hundred operations from each of four written-
  down seeds, then the hashes, the review clocks, one live memory per topic key
  except where a merge said otherwise, every orphaned relation marked, and
  `doctor` healthy. It found §12 on its first run. Twelve seeds have been
  through it; the two things the others turned up were both mistakes in what
  the test asserted rather than in the store, which is the ordinary yield of
  this kind of search and worth saying.

## Where it lives

- `src/memory/model.rs` — the types every surface serialises
- `src/memory/rules.rs` — the vocabulary, the review windows, `is_searchable_kind`
- `src/memory/normalize.rs` — one function per rule, plus the idempotence tests
- `src/store/observations.rs` — the local write path
- `src/store/wire.rs` — the replicated write path, mirroring it

## Related

- [`store-and-schema.md`](store-and-schema.md) — the columns and indexes behind this
- [`search.md`](search.md) — how these fields are matched and ranked
- [`mcp-tools.md`](mcp-tools.md) — the surface that writes and reads them
- [`replication.md`](replication.md) — the second write path §5 refers to
