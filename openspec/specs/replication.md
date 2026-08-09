# Replication

## Purpose

Optional. A second machine, or a shared PostgreSQL peer, holding the same
memories. Everything else in this folder describes a store that works alone;
this describes what happens when it does not have to.

## Behaviour

1. **Local first, always.** A write commits to SQLite and *then* enqueues a
   mutation for the peer. A peer that is unreachable slows nothing down and
   loses nothing: the queue waits.

   Waiting is not free, and the queue drains on nothing but an acknowledgement —
   the prune deletes rows that were acked and have since aged out, so a peer
   that never answers means every row it ever took is still there. Measured:
   twenty memories of about two kilobytes cost 116 KB on disk unenrolled and
   188 KB enrolled, the difference being this journal carrying each write's
   payload a second time. That is the price of not losing a write, and it is a
   price somebody should be able to see.

   Which is why `cloud status` answers `pending_since` beside
   `pending_mutations`. A hundred waiting from this morning is a busy peer and a
   hundred waiting since March is replication that stopped, and the count reads
   identically in both. The date is the oldest unacked mutation, and `null` when
   nothing waits.

2. **The replicated path applies the same rules as the local one.** Every
   normaliser, every redaction, every bound. `store/wire.rs` mirrors
   `store/observations.rs`, `sessions.rs`, `prompts.rs`, and `relations.rs`, and
   a guard test feeds the same dirty input to both and requires the same row —
   four replicated paths once shared a hole that had been fixed on one of them.

3. **Pinning does not travel, and nothing else stays behind.** See
   [`memory-model.md`](memory-model.md) §9. Guarded by comparing the thing
   itself rather than counting what arrived: a memory, its session, its prompt
   and a judged relation are built with every field populated, put through the
   payloads the store actually queued, applied by the code that applies them,
   and compared whole. Four entities, four hand-written column lists at the
   receiving end, and that shape is what once dropped a pin in silence.

   A deletion keeps its kind on the way over. A soft delete arrives soft — the
   peer keeps the row with the same `deleted_at`, so `mem_get_observation`
   answers there the way it answers here — and a hard one arrives hard, with
   the row gone. The `hard_delete` field on the payload is what tells them
   apart, and it is load-bearing: the rest of what a tombstone carries is not,
   because the apply side fills a missing `sync_id` from the mutation's own key
   and decides on the operation rather than on the `deleted` flag. The
   tombstone a project move queues says `hard_delete: false` for that reason,
   so a memory that left a replicated project leaves a tombstone at the peer
   rather than a hole.

4. **A mutation that cannot be applied is deferred, not dropped.** Deferred
   mutations are counted, listed, and replayed; `leteo sync` and `leteo doctor`
   both report how many are waiting.

5. **A lease keeps two machines from syncing the same target at once**, and a
   failure is recorded against the target rather than retried blindly.

6. **Enrolment names a project.** A write enqueued under a name no target is
   enrolled in stops replicating, silently — which is what merging two projects
   used to do.

   And what moving one memory used to do. The queue writes under the project a
   row is in *now*, so `mem_update` changing a memory's project from an enrolled
   one to an unenrolled one queued nothing at all: the peer went on holding it
   under the old name, with the old body, for ever. The answer is not the
   merge's — there the canonical project takes over the source's enrolment,
   because the memories are the same set under a new name, while enrolling a
   destination here would start replicating a project nobody asked to
   replicate. What travels is the only thing true from where the peer stands:
   a deletion under the project it is watching. Only in that direction — into
   an enrolled project the upsert already carries it, and between two enrolled
   projects the row names its own project and the peer follows it.

7. **A reread stays home, and stays.** `review_after` is on no payload: the
   receiving store derives it from the memory's own date and type, which is what
   makes two machines agree that a decision made in January is due in July
   rather than six months after whichever one heard about it last.
   `mark_reviewed` is the one act that moves the clock off that derivation —
   "read today, ask again in six months" — so it is local, because there is
   nothing on the wire to carry it.

   Local is right. Local *and undone* would not be, and that is where this
   differs from a pin: a pin survives because nothing overwrites it, while a
   clock is recomputed on every arrival. An arriving memory of the same type
   leaves a clock that has already been set alone; one whose type changed is a
   different window and is worked out afresh. Without the first half, an old
   decision somebody reread would fall due again the moment any peer touched it,
   and again the time after that — and the suite had nothing on it.

## Known gaps

- **There is no client half of the chunked export protocol.** The server speaks
  it and a peer would answer, but nothing on this side ever starts one. Four
  functions used to sit here waiting to be finished — about 110 lines, no
  caller, and the reason this file's coverage looked worse than its behaviour —
  and they were deleted rather than left: unfinished code that nothing reaches
  reads as a feature to whoever finds it, and costs a coverage number nobody can
  act on. Replication works without it, one mutation at a time over the journal;
  the chunked path is an optimisation for a first sync of a large store, and it
  is a thing to write when there is a peer to measure it against.
- The remaining uncovered paths need a live PostgreSQL peer, so they are
  `#[ignore]`d rather than skipped silently.

## Invariants

- No behaviour exists only on the replicated path. If a rule is worth applying
  when a memory is saved here, it is worth applying when one arrives from
  elsewhere — and the reverse.
- Replication never blocks a local answer.

## Where it lives

- `src/store/wire.rs` — the replicated write paths
- `src/store/replication.rs` — the queue, the leases, the deferred mutations
- `src/sync/`, `src/cloud/` — the transport, the auth, the server half
- `src/sync/tests.rs`

## Related

- [`memory-model.md`](memory-model.md) — the rules §2 requires both paths to share
- [`store-and-schema.md`](store-and-schema.md) — the tables the queue lives in
- [`cli.md`](cli.md) — `leteo sync`, `leteo cloud`, `leteo enroll`
