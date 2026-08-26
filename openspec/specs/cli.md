# Command line

## Purpose

The surface for a person at a terminal and for scripts. Same store, same rules
as [`mcp-tools.md`](mcp-tools.md); different audience, and therefore different
duties about what an answer explains.

## Behaviour

1. **Every command prints JSON on stdout.** Explanations, warnings, and hints go
   to stderr, so a pipe stays parseable and a person still gets told.

2. **Reads are scoped to the current project, and say so.** `search`, `recent`,
   and `context` answer for the project the working directory belongs to and
   report how many memories the rest of the store holds; `--all-projects`
   widens. Before this, 72% of answers on a real store came from somewhere else
   entirely.

3. **An empty answer explains itself, on every read that narrows.** `[]` with
   no reason is the least useful possible reply. `search`, `recent` and
   `context` all say which of the two reasons emptied them — the store has
   never heard of this, or it is filed under another project — sharing one
   sentence with the two tools and the session-start block
   ([`search.md`](search.md) §4). On stderr, and only where the directory chose
   the project: naming one with `--project` is somebody who knows where they
   are looking, and `--all-projects` has already looked everywhere.

4. **`leteo doctor` reports; `leteo doctor --repair` fixes.** The repair restores
   missing full-text triggers, rebuilds the indexes, and recomputes stale hashes,
   reporting `restored_triggers`, `rebuilt`, and `rehashed` beside the ordinary
   report. `--project <name>` adds that project's statistics.

   `--check <code>` selects which verdict comes back and what `healthy` is
   computed from — every check still runs. That is deliberate rather than
   unfinished: the aggregate numbers in the report (`integrity_check`,
   `foreign_key_violations`, the row counts, `pending_mutations`) are fed by the
   checks, and a report that answered `observation_fts_rows: 0` because nobody
   asked about the index would be a worse lie than the time it saves.

   The time it would save, measured on a store of 4,013 memories in a 40 MB
   file: `doctor` costs 315 ms, of which `PRAGMA integrity_check` over the whole
   file is 150 and the three full-text integrity checks are 100. Everything else
   together — the row counts, the foreign keys, reading every body for the hash
   check, the shared topic keys, the triggers, the pragmas — is about 30. So a
   single check could cost a tenth of that, and does not, and this is the note
   somebody should read before changing it.

5. **`leteo setup` installs into an agent, and can uninstall.** It writes only
   what it owns: another tool's hooks in the same configuration file are left
   alone. `--language` alone is a complete command, and so is `--context`.

   Uninstalling removes what it wrote and nothing else, and both halves are
   driven over the whole registry. Ten agents keep their instructions in a file
   that was already theirs, and lose only Leteo's block; three get a file Leteo
   invented and named after itself, and that file goes. Pi has no instruction
   file at all — ten, three and one is the whole registry, and
   `the_registry_splits_three_ways_and_the_counts_are_taken_from_it` is what
   keeps that sentence true when a fifteenth agent arrives. That file goes only
   when nothing else is in it — somebody's own paragraph in
   `leteo-memory-protocol.md` keeps it — and a shared instruction file that was
   there and empty before Leteo arrived is not read as Leteo's. Three of the
   fourteen used to leave a file behind, one of them a Copilot instruction file
   that still applied to every source file and said nothing.

   **An agent gets only the registrations its client can fire.** ZCode holds
   providers, plugins, its own hooks and its MCP servers in one JSON document —
   `~/.zcode/cli/config.json`, verified in the client's own source, servers
   under the nested `mcp.servers`. Its hooks sit under `hooks.events.<Event>`,
   and Leteo prunes nothing there but its own entries when leaving. That client
   supports seven lifecycle events, and neither `SubagentStop` nor `SessionEnd`
   is among them — three of Leteo's five land, with ZCode's instruction file
   telling it to close sessions through `mem_session_summary` itself.
   [`hooks.md`](hooks.md) records why `session-stop` does not move onto `Stop`
   to fill the gap: registered there it ended a session every turn, which broke
   the save reminder once for real.

   Those events sit behind an `enabled` switch that starts off. Leteo turns the
   runner on; where somebody has deliberately turned it off, a typed
   `--hooks` refuses and the wizard installs everything the refusal was not
   about. [`hooks.md`](hooks.md) §20 has that rule and what `doctor` says when
   the switch moves after the fact.

   **Each agent is configured where that agent actually reads.** The path is the
   one taken from the product's own source, not from the shape of its directory:
   the Gemini CLI resolves `~/.gemini/settings.json` on every platform including
   Windows, and loads `GEMINI.md` beside it as context. Writing to
   `%APPDATA%\gemini` instead, or to the `system.md` that is read only under
   `GEMINI_SYSTEM_MD` and replaces the whole system prompt when it is, produced a
   setup that reported success over files the agent never opens.

   **The server is configured to be there when the session opens.** Where an
   agent's format has a choice about it, `setup` takes the one that starts the
   server with the session: Pi's file is read by `pi-mcp-extension`, whose
   `lazy` — its default, and what Leteo used to write — keeps the server down
   until somebody types `/mcp:start leteo`, so the tools were missing from every
   session nobody turned them on in. Memory that has to be switched on by hand
   is not memory.

   No two agents share an MCP configuration or a hooks file. One *instruction*
   file may be shared, and exactly one is: the Gemini CLI and Antigravity both
   read `~/.gemini/GEMINI.md`. Installing both leaves one block rather than two,
   because the block is spliced by marker; uninstalling one leaves the block
   while the other still names Leteo in its own configuration, reports that it
   did with `kept_for`, and takes it away when the last of them lets go.

6. **`leteo export` is this store written down, field for field.** Whatever an
   export contains, an import restores — including pinning
   ([`memory-model.md`](memory-model.md) §9), review dates, the prompt a memory
   answers, and deletions. A backup that silently drops what somebody chose to
   keep in front is a lossy backup, and counting rows cannot see that: the guard
   populates every field a memory can carry, sends it through the JSON, and
   compares the two memories whole.

   An import builds the full-text indexes once at the end rather than row by
   row: the triggers come off inside the same transaction and go back with a
   rebuild before it commits. Every insert otherwise tokenises a title and a
   body three times over, which on a real store — 4,013 memories, 486 sessions,
   1,198 prompts, 326 relations — is 13.3 seconds against 1.5. Inside the
   transaction because a failure has to take the schema back with the rows: an
   import that stopped half way must not leave indexes with no triggers keeping
   them level, which is a store that answers searches with yesterday's words and
   looks fine doing it.

7. **`leteo import --from-engram` adopts an Engram database in place.** It runs
   before anything opens the target, because it replaces the file.

8. **`leteo` with no arguments and a terminal on both ends opens the TUI.**
   Reading keys needs a real stdin, so an interactive flow is offered only when
   stdout *and* stdin are terminals; anything else gets JSON.

9. **`leteo recent` answers about memories, not about sessions.** Session
   summaries are left out by default, the way every other "what happened
   recently" surface leaves them out — the opening block, `mem_context`, the
   memories a prompt hint may name, the widened stages of a search. They were a
   third of the answer on two real projects, seven and eight of twenty.
   `--summaries` brings them back, and the count of what was held back is said
   on stderr when there was any.

10. **`leteo conflicts scan --dry-run` says what applying would do.** The same
    questions, the same numbers, the same cap — only the writes are withheld.
    It used to skip the loop that asks whether a pair is already known, so both
    of the numbers it reported were zero whatever the store held: on a real
    project it previewed 2,400 candidates and 0 already related, where applying
    skipped 299. The preview costs what the apply pays for the same answer.

    **A pair that already carries a verdict is not asked again**, by either
    scan. `find_candidates` hides settled pairs when it is going to file one
    and shows them to a preview — that is what `skip_insert` asks for — so the
    caller has to ask, and the semantic scan did not. Every unasked pair was a
    paid model call answering a question the store had already answered, and
    the answer was written over the one on record: a `supersedes` an agent had
    already settled, downgraded to `related`, takes the caveat off all six
    surfaces that carry it. Two of the first hundred pairs on a real store,
    which is small and is not the point — that store holds 255 judged pairs and
    the scan walks the newest memories first, so the share grows with every
    scan somebody runs. The number is reported as `already_judged`, beside the
    `already_related` the other scan reports, and a pair merely *proposed* is
    still a question: only a judged verdict counts.

11. **A store somebody else is writing to is not a broken one.** It answers with
   one sentence saying the call did nothing, nothing is half-written, and it can
   be done again — the same sentence the tools and the hooks use. Every other
   failure keeps its whole cause chain, because a person debugging a real fault
   wants it; this one is not a fault, and it printed `Error code 5: database is
   locked` three times over.

12. **`leteo save` records the question it answers, by the same rule the tool
   uses.** The session's last prompt, and then the project's inside a window for
   a save that named no session — one rule, in the store, read by both doors
   ([`mcp-tools.md`](mcp-tools.md) §6). This wrote nothing at all: the same
   memory recorded its question or did not depending on which door it came
   through, and the terminal was the silent one.

13. **`leteo context` is the configured size, like everything else that opens a
   context.** Three surfaces build it — the session-start hook, `mem_context`,
   and this — and this one used a constant twenty while the other two read
   `context_size`, whose default is fifty. An untouched installation showed a
   person at a terminal 40% of what their agent was handed, and
   `leteo setup --context deep` moved two of the three. `--limit` still outranks
   the setting, and here it outranks it without a ceiling — which is the one
   place these three surfaces deliberately part company.

   `mem_context` caps its budgets, because what it hands back goes into an
   agent's context window and a reply that pushes the useful part out of one has
   failed at the thing the tool is for
   ([`mcp-tools.md`](mcp-tools.md) §3). A terminal has no window: `--limit 9999`
   is a person asking for everything, into a pipe. So the same number answers
   differently by design — 80 memories and 43.7 KB through the tool, whatever
   was asked for and 99 KB here — and the two are not the same product either.
   This prints the rendered block, the same text the session-start hook
   injects; the tool answers with the structured lists. What they must agree on
   is *which* memories, and they do: over the same store and budget, the same
   fifty in the same order.

## Invariants

- Every documented command exists, and every command is documented. A test in
  `tests/documented_commands.rs` walks the README against the parser.
- A sentence printed to a person carries no source indentation from the Rust
  string it was formatted in. A guard in `tests/repository_guards.rs` reads
  every string literal under `src/` and fails on either way of breaking one
  across two source lines.
- The CLI opens the store no earlier than the work needs it — see
  [`hooks.md`](hooks.md) §3.

## Where it lives

- `src/cli/args.rs` — the parser, and the single list of hook event names
- `src/cli/mod.rs` — the commands
- `src/cli/projects.rs` — read scoping and project resolution
- `tests/cli_integration.rs`, `tests/documented_commands.rs`,
  `tests/repository_guards.rs`

## Related

- [`mcp-tools.md`](mcp-tools.md) — the same operations for an agent
- [`store-and-schema.md`](store-and-schema.md) — what `doctor` checks and repairs
- [`hooks.md`](hooks.md) — what `setup` installs
- [`replication.md`](replication.md) — `leteo cloud` and `leteo sync`
