# Working on Leteo

Local-first persistent memory for coding agents: one Rust binary over one SQLite
database. Read [`openspec/project.md`](openspec/project.md) first — it says what
the crate is and how it is laid out.

**Everything written into this repository is in English** — code, comments,
commit messages, tests, and the documents under `openspec/` — whatever language
the conversation about it happens in.

## Keep `openspec/` current

`openspec/` is the written half of this project: one file per capability, saying
what the system guarantees and why. It is maintained as part of the work, not
after it.

- **A change to a promise and its spec edit belong in the same commit.** A
  commit that leaves a spec describing behaviour the code no longer has is worse
  than no documentation, because a reader will believe the wrong one.
- **Keep it structured.** Each spec has the same five sections — Purpose,
  Behaviour, Invariants, Where it lives, Related — and the requirements under
  Behaviour are numbered so they can be cited (`search.md §3`). Numbers are
  stable; a requirement that stops holding is rewritten in place, never silently
  renumbered.
- **Keep it related.** Every file ends with links to its neighbours, so any file
  is a way in to all of them. A fact that belongs in two specs lives in one and
  is linked from the other — stated twice, it will be edited once.
- **Keep it readable.** A spec longer than one sitting is two specs.

[`openspec/README.md`](openspec/README.md) lists exactly which kinds of change
require a spec edit. Read it before adding a tool, a command, a flag, a
`doctor` check, a migration, an output field, or a default.

## Building and testing

```sh
cargo fmt --all
cargo clippy --all-targets
cargo test              # unit tests, plus the integration tests in tests/
cargo build --release
```

All three must be clean before a commit. `cargo test --lib` alone is not enough:
it skips `tests/`, which is where the surface-level guards live.

## Rules that have cost this codebase real bugs

1. **No test touches a real store.** Unit and integration tests create a
   temporary database. `tests/repository_guards.rs` fails the build on an
   absolute `.db` path, a `home_dir()` join, or a data directory used near
   anything that opens a store. Exploratory work against a copy is fine; a
   scheduled test against the user's database is not.
2. **Fix the sibling too.** Nearly every defect found here has had one: the
   prompt was bounded and the summary next to it was not; four of six tools
   carried a memory's caveats; one of four replicated write paths was redacted.
   After fixing something, look for the field, path, or surface beside it.
3. **A rule lives in one place.** Types, review windows, hook event names, index
   names, trigger names — each is one list that the behaviour and its tests both
   read. A hand-written second copy is how `policy` kept a review window that
   nothing could ever fire.
   One exception, licensed and written down rather than left to look like the
   defect: a *released* migration may freeze a copy of what it needs, because it
   must give every database the same answer whenever it happens to run.
   `openspec/specs/memory-model.md` records the only one there is.
4. **A limit that is published is the limit that is applied.**
5. **Say what could not be done.** An empty answer, a busy store, a check that
   could not run — each says which it is. Reporting the nearest named state
   instead is how a healthy store gets called corrupt.
6. **A guard is verified by breaking what it claims to protect.** A test that
   passed the first time it ran may be watching nothing. Break exactly the
   behaviour its own sentence names, confirm it fails, and restore — `git diff`
   of that file must come back empty. Breaking something *near* it is worse than
   not checking: the intersection in `engram.rs` was "verified" by reversing two
   sides that produce the same set, which passed, and the comment explaining why
   the order mattered was wrong. Six guards of one session were put through this
   afterwards; all six failed as they should, and the checking is what makes
   that sentence worth writing.
7. **Measure before claiming.** Timing does not transfer between SQLite builds;
   ranking does; query *construction* does not transfer at all. A benchmark that
   does not go through this binary's own query builder measures something else.

## Comments and commits

Comments say *why*, with the measurement or the failure that motivated the code
where there was one. They are prose, in full sentences, aimed at whoever changes
this next.

There is no second kind. A comment that narrates what the next line does,
restates an item's name, or captions a test is deleted on sight — the code is
already saying it, and the copy drifts. A review that accepts one has accepted
the drift. What stays is narrow: measurements, failure histories, ordering and
locking invariants, workarounds for compiler/SQLite/OS quirks, and attribution.

One exception, and it is not a comment: the `///` docs on the `schemars` types
in `src/mcp/` are the `description` fields of the JSON Schema agents read. They
are published data; deleting one breaks the tool surface, which is how you know
you were looking at the wrong category.

**No test reads a comment** — not its wording, and not a count or a list it
states either. Two guards in `tests/repository_guards.rs` used to, over the
sentences saying how many tests need a database and over the coverage note in
`ci.yml`, and both were deleted along with the counts they held. A comment that
needs a guard to stay true is making a claim it should not make: drop the claim
rather than check it. Those sentences now name no number, and `cargo test -- --ignored`
is the list.

Commit subjects are a sentence about what changed, not a label:
`Notice a hash that has stopped describing its memory, and put it back`. The
body says what was wrong, what it cost, and how it was measured.

## Migrations

Append-only. A released migration is never edited — databases that already ran
it will not run it again. Add a new file, bump `SCHEMA_VERSION`, and rebuild the
full-text indexes if the migration rewrote a column one of them carries — a
migration that touches only a column no full-text index carries, as 18 does with
`review_after`, does not need to. See
[`openspec/specs/store-and-schema.md`](openspec/specs/store-and-schema.md).

## Attribution

Leteo is a reimplementation of Engram. The Engram attribution in `NOTICE` and
`LICENSE` stays — it is an MIT requirement, not a courtesy.
