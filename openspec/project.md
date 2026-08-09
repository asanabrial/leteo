# Leteo

Local-first persistent memory for coding agents. A single Rust binary over one
SQLite database: agents save what they learn and find it again in a later
session, on the same machine, with no service in between.

It is a reimplementation of [Engram](https://github.com/engram-design/engram)
and can adopt an Engram database in place — see `NOTICE` for the attribution
that stays.

## What the binary is

One executable, `leteo`, with several faces onto the same store:

| Face      | Entry point         | Who talks to it                                       |
| --------- | ------------------- | ----------------------------------------------------- |
| MCP server| `leteo mcp`         | agents, over stdio — the main surface                  |
| Hooks     | `leteo hook <event>`| the agent's own lifecycle, unattended                  |
| CLI       | `leteo <command>`   | a person at a terminal, and scripts                    |
| TUI       | `leteo` (no args)   | a person configuring it                                |

All four open the same database and go through the same store layer. A rule
enforced in only one of them is a rule that does not exist — see
[`specs/memory-model.md`](specs/memory-model.md) §5.

## Crate layout

```text
src/
  store/       the database: schema, migrations, queries, diagnostics
    search.rs      three-stage full-text search and ranking
    schema.rs      the shape every database converges on, and the migrations
    diagnostics.rs doctor's checks and its repairs
    wire.rs        the replicated write paths, mirroring the local ones
  memory/      the model, and the rules that hold whatever the model touches
    model.rs       the types every surface serialises
    normalize.rs   one place per normalisation, shared by every write door
    rules.rs       the vocabulary of types, and how long each stays trustworthy
  mcp/         the MCP server: tools, typed output, parameter parsing
  hooks/       the five lifecycle events, their budgets, and what they emit
  cli/         the command-line surface
  recall/…     recall.rs, the opening context an agent is handed
  sync/, cloud/  optional replication to a PostgreSQL peer
  setup/, tui/   installing into an agent, and the interactive configuration
migrations/    the SQL, embedded at build time; never edited once released
tests/         integration tests that run the built binary
openspec/      these documents
```

## Invariants of the whole system

1. **The store is the only truth.** Nothing is cached across processes; every
   surface reads the database each time it answers.
2. **A hook never blocks its agent.** Every hook has a budget shorter than the
   patience of the agent that registered it, and gives up in time to answer.
   See [`specs/hooks.md`](specs/hooks.md) §2.
3. **Every write door normalises.** MCP, CLI, hooks, and replication all reach
   the store through the same normalisation, so the same input produces the same
   row whichever door it came in by.
4. **Migrations are append-only.** A released migration is never edited; a
   database that already ran it will not run it again.
5. **A limit that is published is the limit that is applied.** Where an answer
   is truncated, the number in the output is the number the truncation used.

## Related

- [`specs/memory-model.md`](specs/memory-model.md) — what a memory is
- [`specs/store-and-schema.md`](specs/store-and-schema.md) — how it is stored
- [`specs/search.md`](specs/search.md) — how it is found again
- [`specs/mcp-tools.md`](specs/mcp-tools.md) — the agent-facing surface
- [`specs/hooks.md`](specs/hooks.md) — the unattended surface
- [`specs/cli.md`](specs/cli.md) — the human surface
- [`specs/replication.md`](specs/replication.md) — the optional second machine
