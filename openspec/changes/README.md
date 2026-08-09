# changes

Proposals: a change described before it is built.

Not every change needs one. A bug fix goes straight into the code with its spec
edit beside it. A file belongs here when the change is worth arguing about
first — a new capability, a promise being dropped, a default that many things
depend on, anything that would edit more than one spec.

## Shape

One file per proposal, named for what it does: `search-by-recency.md`, not
`proposal-3.md`.

```markdown
# <what it does>

## Now
What the current behaviour is, and what is wrong with it. Cite the spec:
`search.md §3`.

## Proposed
What it would do instead. Concretely enough to implement from.

## Cost
What it makes slower, larger, or harder. Every change has one; a proposal that
claims none has not been examined.

## Specs to edit
The files and sections this would change.
```

## Lifecycle

A proposal is deleted when it ships — the spec edits land with the change, and
`git log` keeps the argument. A proposal that is rejected is deleted too, with
the reason in the commit message that removes it. Nothing stays here to rot.
