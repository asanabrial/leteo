# A revision nobody can undo

## Now

Two writes replace a memory's text in place, and neither keeps what was there.

`mem_update` is the obvious one: it sets `title`, `content`, `type`, `project`,
`scope` and `topic_key` on the row, and the previous values are gone. `mem_save`
does the same whenever the `topic_key` matches a memory that already exists —
which is what the key is for ([`memory-model.md`](../specs/memory-model.md)
§10), and which the reply reports as `status: "revised"`.

Measured on a copy of a real store, driving the built binary over the protocol:
after a revision the previous body is not in the row, not in a tombstone, not in
the replication queue, and not in the full-text index. `revision_count` goes
from 1 to 2, which is a count of versions nobody can read.

The contrast is what makes this worth writing down. `mem_delete` is
*recoverable*: it writes `deleted_at` by default and leaves the body in the row,
and `hard_delete` is the caller asking for the other thing. The two writes that
cannot be undone are the two that look like ordinary saves.

It matters most for exactly the memories the store exists to hold. A `topic_key`
is put on a decision, a policy, an architecture note — the things that evolve
over months — and the key is right: it keeps the current answer in one place
instead of six near-duplicates. What goes with it is the ability to ask what the
answer used to be and when it changed, and the only recovery from a bad write.
An agent that revises the wrong id has destroyed something, in a tool whose
whole promise is that it remembers.

## Already done

The annotations now say so. `mem_update` and `mem_save` declare
`destructive_hint = true`, held by a guard that drives each write and asks
whether the previous body is still findable
(`no_tool_that_replaces_stored_text_calls_itself_additive`). Before that both
declared they made only additive updates, and the tool that soft-deletes was the
only one warning about destruction.

That is honest metadata about a behaviour that may still be the wrong one.

## Proposed

**Keep a revision history**: an `observation_revisions` table holding the
previous `title`, `content` and `type` with the timestamp, written inside the
same transaction as the revision. `revision_count` becomes a number somebody can
act on, and `mem_timeline` gains an obvious question to answer.

The cheaper alternative, if the measurement below argues for it: **keep only the
last one**, as two columns on `observations` rather than a table. Enough to undo
a mistake, not enough to answer "when did this change". It covers the recovery
case, which is the one with no answer at all today.

The third option is to **leave it and say so** — documented in
[`memory-model.md`](../specs/memory-model.md) §10 as a deliberate loss rather
than an omission. Defensible, since a memory store is not a version control
system, but it should be a sentence somebody wrote on purpose.

## Cost

Bytes, on every revision. This store holds 4,013 memories with a median body
well over a kilobyte, and a full history doubles the storage for anything
revised often. The revision *rate* is not known and has to be measured before
choosing — count the rows with `revision_count > 1` and look at the distribution
above it, because the two options differ by exactly that number.

A write also gets one more insert inside the transaction that already holds the
row, which is the cheapest part of it.

And a history is a second place private text can survive a redaction. Whatever
the write door removes on the way in has to be removed here too, or the table
becomes the copy that still has it.

## Open question

Whether replication carries a revision or a replacement. A peer that only ever
receives the current text cannot reconstruct a history it was not sent, so
either the payload grows or the history is local-only — and a local-only history
is a different promise from the one this would appear to make.

## Specs to edit

- [`memory-model.md`](../specs/memory-model.md) §10 — what a revision keeps
- [`mcp-tools.md`](../specs/mcp-tools.md) — the reply that reports `revised`
- [`store-and-schema.md`](../specs/store-and-schema.md) — the table and its
  migration
- [`replication.md`](../specs/replication.md) — if a revision crosses the wire

## Related

- [`memory-model.md`](../specs/memory-model.md) — topic keys and revision
- [`store-and-schema.md`](../specs/store-and-schema.md) — where a table would go
