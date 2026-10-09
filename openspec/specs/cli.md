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
   missing full-text triggers, rebuilds the indexes, recomputes stale hashes,
   folds the memory types an older adoption copied verbatim through
   `normalize::kind`, and drains the spool a busy hook kept captures in,
   reporting `restored_triggers`, `rebuilt`, `rehashed`, and `folded_types`
   beside the ordinary report. `--project <name>` adds that project's
   statistics.

   The spool is a directory beside the database, not a table, so it has its own
   check: `hook_spool` counts the captures waiting for a later open and names the
   age of the oldest, passing when there are none and failing with
   `leteo doctor --repair` as the remedy otherwise
   ([`hooks.md`](hooks.md) §22, [`store-and-schema.md`](store-and-schema.md)
   §19).

   `--check <code>` runs **only** that check. The whole point of asking about
   `busy_timeout` is not paying for `PRAGMA integrity_check`, which on a 96 MB
   store is several seconds the one answer does not need. The counts and the
   pragmas are gathered either way — they cost a few milliseconds and answer the
   same question under `--check` as under `doctor` — while a check that did not
   run leaves its own aggregate absent rather than zero: `integrity_check` and
   `foreign_key_violations` are omitted, not empty, so no report reads as "the
   index holds nothing" when the index was never looked at.

   Every check carries a severity — `error`, `warning` or `info` — and `healthy`
   means no `error`. A missing or unverified model, a word no filter can name, a
   journal mode that is not `wal`, a spool with captures waiting, settings being
   read past: each is a warning that names the capability lost and leaves the
   store healthy. `leteo doctor` exits non-zero when an `error` exists and zero
   otherwise, so a script can act on the code and a person on the report beside
   it.

   `doctor` also reports a configured command whose executable is gone. Every
   supported agent's MCP configuration and hooks file is read for the Leteo
   command, and an absolute path that no longer exists is named with the file
   that holds it — in `missing_binaries`, and in `issues`, which makes the report
   unhealthy. A wrapper like `npx` is resolved through `PATH` and is not judged.
   This is the failure the hooks themselves cannot report: a hook that fails
   says nothing by design.

5. **`leteo setup` installs into an agent, and can uninstall.** It writes only
   what it owns: another tool's hooks in the same configuration file are left
   alone. `--language` alone is a complete command, and so is `--context`.

   **The path written is the stable one where a package manager keeps one.**
   `current_exe()` resolves through `<prefix>/bin/leteo` into
   `<prefix>/Cellar/leteo/<version>/bin/leteo`, and `brew upgrade` removes that
   versioned directory — after which every hook and the MCP server fail, and a
   hook that fails says nothing by design. So the canonical path is put back to
   the stable link when it sits under a versioned package directory — Homebrew's
   `Cellar/<formula>/<version>`, or mise's and asdf's `installs/<tool>/<version>`
   — and that link exists. An ordinary install keeps its canonical path.

   Uninstalling removes what it wrote and nothing else, and both halves are
   driven over the whole registry. Eleven agents keep their instructions in a file
   that was already theirs, and lose only Leteo's block; three get a file Leteo
   invented and named after itself, and that file goes. Pi has no instruction
   file at all — eleven, three and one is the whole registry, and
   `the_registry_splits_three_ways_and_the_counts_are_taken_from_it` is what
   keeps that sentence true when a sixteenth agent arrives. That file goes only
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

   **A file edited by line is edited in the notation it already uses.** DeepSeek
   Harness has no per-project configuration for an installer to write, so setup
   edits one machine-global patch layer, `$DSH_HOME/cordis.patch.yml`, which
   every profile composes. That file is a top-level YAML array, and an array has
   two notations. Leteo writes the block one, so the flow one has to be
   recognised rather than appended to: the harness ships the layer holding `[]`,
   and `[]` followed by `- insert:` is two nodes in one document, which no
   parser accepts. Appending it left people unable to open a session at all —
   not a failed install, a broken harness.

   So `[]` is replaced when the first row arrives, and put back when the last
   one leaves, because a file of comments alone is `null` rather than the empty
   array its own header says it is. A flow array with entries in it, or a
   mapping, is refused with the file named: merging into either needs the YAML
   parser this crate deliberately does not carry, and writing a document nothing
   can read is worse than saying so. The same judgement governs the Codex TOML
   and every JSON config — what is owned is spliced, and what is not owned
   survives byte for byte or the write does not happen.

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
   answers, deletions, and the version history a content-changing write kept
   ([`memory-model.md`](memory-model.md) §14). A backup that silently drops what
   somebody chose to keep in front is a lossy backup, and counting rows cannot
   see that: the guard populates every field a memory can carry, sends it
   through the JSON, and compares the two memories whole.

   An import builds the full-text indexes once at the end rather than row by
   row: the triggers come off inside the same transaction and go back with a
   rebuild before it commits. Every insert otherwise tokenises a title and a
   body three times over, which on a real store — 4,013 memories, 486 sessions,
   1,198 prompts, 326 relations — is 13.3 seconds against 1.5. Inside the
   transaction because a failure has to take the schema back with the rows: an
   import that stopped half way must not leave indexes with no triggers keeping
   them level, which is a store that answers searches with yesterday's words and
   looks fine doing it.

   Both shapes of an Engram direct backup are read. Engram moved its file to
   `0.2.0` in v3.0.0 — relations lost their local `id` and name their
   supersession chain by `sync_id`, prompts gained an inbox identity, and a
   `prompt_tombstones` list appeared — and Leteo reads that alongside the
   `0.1.0` it still writes. The tombstones land in `prompt_deletions`, the home
   a deletion already has, so a prompt the source deleted is not resurrected by
   a later import. The inbox identity and the relation supersession chain have
   no column in this model and are not carried; an unknown version is still
   refused by name rather than half-read.

7. **`leteo import --from-engram` adopts an Engram database in one
   transaction, and never deletes what is there.** The source is snapshotted
   and the copy runs under a single `BEGIN IMMEDIATE` on the target, so a
   failure leaves the target exactly as it was and the command is run again.
   A target that already holds memories in any mapped table — sessions, prompts
   or observations, not only observations — is refused, naming what it holds;
   it used to be deleted whenever its `observations` happened to be empty.

8. **`leteo` with no arguments and a terminal on both ends opens the TUI.**
   Reading keys needs a real stdin, so an interactive flow is offered only when
   stdout *and* stdin are terminals; anything else gets JSON.

9. **`leteo recent` answers about memories, not about sessions.** Session
   summaries are left out by default, the way every other "what happened
   recently" surface leaves them out — the opening block, `mem_context`, the
   memories a prompt hint may name, the widened stages of a search. They were a
   third of the answer on two real projects, seven and eight of twenty.
   `--summaries` brings them back, and the count of what was held back is said
   on stderr when there was any. A memory a judged `supersedes` points at is
   left out of `recent` too, the way `search` and `context` already leave it
   out — `--summaries` is about session summaries and does not bring it back
   ([`memory-model.md`](memory-model.md) §13).

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

14. **`leteo consolidate` merges several memories into one, and files the
    replacement where the sources are.** `leteo consolidate <ids…> --title …
    --content …` writes the replacement and records a judged `supersedes`
    relation to each source, in one transaction — the same store method
    `mem_consolidate` calls, so the CLI and the tool cannot disagree about what
    a merge is. It takes no project assertion: a person named the ids, and the
    replacement is filed where the first source is rather than where the command
    happened to be run from, so a merge does not move the family to the
    directory's project. The reply is the same outcome the tool serialises, with
    `sources` naming the ids that were replaced.

15. **`leteo search` goes on to look by meaning when the words cannot answer,
    and says so.** It asks the same search `mem_search` asks, with the stage on
    unless the `semantic_search` setting beside the database says `false`
    ([`search.md`](search.md) §15). A result the stage added carries
    `"semantic": true` in the JSON on stdout, and stderr carries the sentence
    `mem_search` answers with — that such a result may contain none of the words
    asked for — in the place the relaxed-answer sentence would be. There is no
    flag: the setting is the switch, and it is read on every search, so editing
    the file takes effect on the next command. The first search that reaches the
    stage on a store embeds what is in scope, which takes about half a second per
    four thousand memories and is paid once. It needs the model, which is a file
    and not part of the binary (16); with no verified model the search is the
    lexical one, unchanged.

16. **`leteo model install` puts the semantic model where Leteo looks for it, and
    `leteo setup` runs it when it is not there.** One command for every install
    that arrived without the model -- `cargo install`, a source build, a distro
    package, a manager added later -- with no case for any of them. It downloads
    the three files from the GitHub release whose tag is the binary's version
    (`--url` or `LETEO_MODEL_URL` names another directory of assets), or copies
    them from a directory with `--from`, which touches no network. Every file is
    checked against the SHA-256 compiled into the binary before anything is
    written, the three go into a directory of their own, and that is renamed into
    `<data dir>/model/`; any refusal leaves what was installed as it was and exits
    non-zero. The reply is JSON naming the directory and the files. `setup` runs
    the same download when no location in [`search.md`](search.md) §15 holds a
    verified model, unless `semantic_search` is false or the run is a `--dry-run`,
    and never fails for want of it: it says on stderr what happened and what to
    run. `uninstall` removes the model from the places the binary looks
    for it ([`search.md`](search.md) §15) except `LETEO_MODEL_DIR`, which names a
    directory somebody else chose: the files of `MODEL_FILES` by name *and* by
    content, then `model/` and `share/leteo` if that left them empty, never
    `share/` or the binary's directory. A file is deleted only if it is a regular
    file whose SHA-256 is its pin, so a `config.json` somebody else keeps there,
    and the model an older release installed, are kept and named in the report's
    `remaining`; a location that is itself a symbolic link is not followed, and
    is named too. A directory that still holds something else -- `model/` or
    `share/leteo` -- is kept and named with what it holds, and one that could not
    be removed while empty is named with the I/O error and not with a stranger. A
    `share/leteo` that is the data directory is never treated as the installer's.
    Without `--yes`, `uninstall` is the preview: nothing is touched and `model_files` lists what would go, by the
    same rule. `scripts/uninstall.sh` and `uninstall.ps1` repeat the removal by
    name for a binary that is gone or could not start, and leave the model files
    to the binary whenever it ran, whatever it exited with. Ran is decided by
    what the binary printed, and not by an exit code or an exception type: dash
    re-runs an exec that failed with ENOEXEC as a script, and exits 2 or 0
    without the binary having run. Either of two lines counts: `leteo uninstall:
    started`, which `uninstall --yes` prints on stderr before it judges any model
    file (`setup::UNINSTALL_STARTED`), or the `"model_removed"` line of the JSON
    report on stdout, which is how a binary built before the marker existed
    shows it ran -- an older archive can leave one beside a newer script, and
    that binary may have kept a file the by-name removal must not take. A test
    keeps both scripts' copies equal to what the binary prints. Neither line
    means it could not start. Output of both streams is shown together. A
    directory they keep is reported as kept,
    with the binary's report as the reason, not as holding strangers, and only
    when there was a report. Both scripts retry the data files by name whenever
    the binary did not finish with exit 0, which undoes no judgment because the
    binary and the scripts name those files identically.
    `tools/semantic/check_install.sh` runs both scripts behind a binary that
    ran, kept a file and failed, behind one that started and failed before
    removing anything, behind a pre-marker one that ran and failed, and behind
    one that cannot start; the shell script also
    behind an executable that is no program, under dash where it is installed.
    The PowerShell flows need `pwsh`, and without it the check exits 2.

17. **`scripts/build-install.sh` and `build-install.ps1` install the checkout
    they sit in.** They are the developer's counterpart to the release
    installers: they build with the pinned toolchain (`rustup run 1.97.0 cargo
    build --release --locked`), install with `cargo install --root <root> --path .
    --locked --force`, where the root is `CARGO_INSTALL_ROOT`, then `CARGO_HOME`,
    then `~/.cargo`, made absolute, and never the release installers' directory.
    They require the installed executable to exist, then run `leteo --version`,
    `leteo model install --from <checkout>/assets/model` (§16: an unreleased build
    has no tag to download from), `leteo setup <agent>` for each agent and
    `leteo doctor`. `leteo setup` with no agent is not used to configure: off a
    terminal it only lists the agents, and on one it is a wizard. The scripts
    name the agents instead, and by default only those that already have Leteo
    configured, so a rebuild repoints existing entries and never creates one for
    an agent that has none. They ask the binary: `leteo uninstall` without
    `--yes` is a preview that changes nothing, and its per-agent `was_configured`
    is the `is_configured` check `setup` uses (§5). That it removes nothing is
    the command's own rule, not the scripts'; they only refuse to read a reply not
    marked `dry_run` as a list of agents. No configured agent is reported and is not an error.
    `LETEO_SETUP_AGENTS` names the agents instead; empty or only blanks is unset,
    and the exact word `none`, alone, skips the step. A reply with no agent
    entries at all is an error naming `leteo uninstall`, not "no agent
    configured". The agent step is plain `setup <agent>`, the MCP entry only, and
    it resets that entry's `--tools` and `--project` to the defaults. A typed
    `--instructions` or `--hooks` is refused for an agent that cannot take it and
    one refusal would end the run. Every failure names the command and exits
    non-zero.

18. **`leteo uninstall` removes Leteo, and never a file it did not write.**
    Without `--yes` it is the preview: nothing is touched, and the report is what
    would go. With it, every agent it configured, the store, its `-wal`/`-shm`
    sidecars, `settings.json`, `cloud.json` and the `hooks` reminder clocks go,
    and the model goes by the rule in §16. `--keep-data` is the one flag: it
    leaves the data directory alone — the store, the settings, and any
    `leteo.db.pre-schema-N` copy a migration left
    ([`store-and-schema.md`](store-and-schema.md) §18) — while the binary and
    every agent still go. Nothing is matched by a broad prefix: the store and its
    sidecars are named exactly, so a hand-made `leteo.db.bak-before-migrate` —
    the name the upgrade notes used to ask for — is kept, and so is a
    `pre-schema` copy, which is the only way back from a one-way migration. The
    report's `data_kept` says a kept store was kept on purpose, and `complete()`
    reads that as finished rather than as a partial removal. A directory that
    still holds something not named here is kept and named, never emptied.

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
- `scripts/build-install.sh`, `scripts/build-install.ps1` — build and install
  the checkout (§17)
- `tools/build-install/check.sh` — runs both behind stand-in `rustup`, `cargo`
  and `leteo` (§17)
- `src/cli/projects.rs` — read scoping and project resolution
- `tests/cli_integration.rs`, `tests/documented_commands.rs`,
  `tests/repository_guards.rs`

## Related

- [`mcp-tools.md`](mcp-tools.md) — the same operations for an agent
- [`store-and-schema.md`](store-and-schema.md) — what `doctor` checks and repairs
- [`hooks.md`](hooks.md) — what `setup` installs
- [`replication.md`](replication.md) — `leteo cloud` and `leteo sync`
