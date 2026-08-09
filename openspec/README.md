# openspec

The written half of Leteo. Every capability the project promises is described
here in prose, in one file per capability, at the level a reader needs to use it
or to change it without reading the whole crate first.

This is not generated documentation. Rustdoc already says what a function takes
and returns; these files say what the *system* guarantees, why, and what would
break if a guarantee were dropped. When the two disagree, the code is the truth
and the spec is a bug.

## Layout

```text
openspec/
  README.md      you are here — the conventions, and the contract to keep them
  project.md     what Leteo is, how the crate is laid out, what is invariant
  specs/         one file per capability, each self-contained and cross-linked
  changes/       proposals: a change described before it is built
```

## How a spec is written

Every file in `specs/` has the same five sections, in this order:

| Section          | Answers                                                        |
| ---------------- | -------------------------------------------------------------- |
| **Purpose**      | What this capability is for, in two or three sentences.          |
| **Behaviour**    | Numbered requirements. One promise each, stated as a fact.       |
| **Invariants**   | What must stay true across every path, including the ones added later. |
| **Where it lives** | The files that implement it, and the tests that hold it to this. |
| **Related**      | Links to the other specs this one touches, and why.              |

Requirements are numbered so a commit message, a test name, or a proposal can
point at one: `search.md §3` is a citation, `"the widening rule"` is a guess.
Numbers are stable — a requirement that no longer holds is rewritten in place or
struck through with a line saying when and why, never silently renumbered.

## The contract

**These files are part of the change, not a follow-up to it.** A commit that
alters a promise and leaves its spec describing the old one has left the
repository in a state where two sources disagree and neither says which is
newer. That is worse than no documentation at all, because a reader will believe
the wrong one.

Concretely, a change to any of these belongs in the same commit as its spec
edit:

- a tool, command, hook event, or flag added, removed, or renamed
- an output field added, removed, or given a different meaning
- a default, budget, limit, or timeout changed
- a normalisation, ranking, or filtering rule changed
- a schema version, migration, or `doctor` check added
- an error code or its recovery path changed

**Keep them structured and related.** A spec that grows past what one sitting
can read is two specs. A statement that belongs in two files goes in one and is
linked from the other — a fact stated twice is a fact that will be edited once.
Every file ends with its **Related** section pointing at its neighbours, so any
file is a way in to all of them.

**Write in English.** The commit log, the code comments, and these specs are all
in English, whatever language a conversation about them happens in.

## changes/

A change worth arguing about before it is built gets a file in `changes/` first:
what is wrong now, what the new behaviour would be, what it costs, and which
specs it would edit. When the change ships, the spec edits land with it and the
proposal is deleted — it is scaffolding, and `git log` keeps the history.
