# A personal memory that follows you between projects

## Now

`scope` is one of `project`, `personal`, or `global`
([`memory-model.md`](../specs/memory-model.md) §11). Only the first of those
does anything. A memory is filed under the project the directory resolved to
whatever its scope, and every read narrows by project — so a memory somebody
marked *personal*, meaning "this is not about the project", is visible only
while standing in the project they happened to be standing in when they saved
it.

Scope changes nothing else either: it is not consulted by replication, by
export, or by the opening context. It is a partition key beside `project` for
deduplication and topic-key revision, and a filter nothing sets.

On a real store of 3,948 memories there are 31 personal ones, spread over seven
projects, and their contents are what the word suggests — the machine, the
operator, the tools:

```text
  [ledgerly/discovery] Found plaintext GitHub token in Codex config
  [ledgerly/preference] Exclude GGA and Herdr from tooling
  [ledgerly/preference] Keep operator configuration private
  [bench-pc/config]       (seven about this machine)
  [leteo/config]         Migrated fully from Engram to Leteo
```

A token found in a configuration file is not a fact about `ledgerly`. It is
findable only from `ledgerly`.

## The obvious version, and why the numbers rule it out

Let a project-narrowed read also return anything whose scope is not `project`:

```sql
WHERE (project = ?1 OR scope <> 'project')
```

Measured against that store, taking the twenty most recent memories each
project's opening block would list, before and after:

```text
  project                          slots taken by another project's personal memories
  almanac                          20 of 20
  papermill                        20 of 20
  bulwark                          20 of 20
  acme-seo                         20 of 20
  bench-pc                         14 of 18
  example-school.com               13 of 17
  nas.archive                      10 of 20
  wordsmith                        7 of 20
  engram                           2 of 20
  ledgerly, task-board, trailmark  1 each
  warden, leteo                   0
```

Four projects lose their opening block entirely, and they are the quiet ones —
the projects with the least memory, which need theirs most. Recency is why: a
personal memory saved last week outranks everything a project that has been
idle for a month ever wrote. The rule is right and the mechanism is wrong.

## Proposed

Treat a cross-project personal memory the way a pinned one is already treated:
**listed on top of the budget, capped, and never competing on recency.**
`recall.rs` already does this for pinned entries — they are listed first and do
not spend the recent-memory budget — so the shape exists and is tested.

- A read narrowed to a project returns that project's memories exactly as it
  does now.
- Ahead of them, at most **three** memories whose scope is not `project`,
  most recent first, drawn from the whole store.
- Deduplicated against the pinned list and against the project's own memories,
  so nothing is listed twice.

Three, because the cap has to be small enough that the four quiet projects
above lose three slots rather than twenty, and because 31 memories across a
year is the rate this actually accumulates at.

## Cost

- Three slots of every opening block, on every project, forever. At `slim` that
  is 15% of the block.
- One more query per opening context. It is indexed by `created_at` the same
  way the recency list is, and the opening block costs 18–22 ms today, so this
  is not where the time goes — but it should be measured rather than assumed.
- A user who has never used `personal` pays nothing, because there is nothing
  to list.

## Open question

`global` is accepted by the normaliser, means nothing, and no memory on the
real store carries it. Either it is what this proposal calls "not `project`"
and `personal` should be the machine-and-operator scope beneath it, or it
should be refused on the way in. Deciding that is part of this change.

## Specs to edit

- [`memory-model.md`](../specs/memory-model.md) §11 — what scope means
- [`search.md`](../specs/search.md) §7 — what a project-narrowed read returns
- [`hooks.md`](../specs/hooks.md) §4 — what the opening block is made of
