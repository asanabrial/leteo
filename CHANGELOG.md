# Changelog

All notable changes to Leteo are documented in this file.

## [Unreleased]

### Added

- **The cloud dashboard can be browsed, read-only.** An administrator signed
  in to `leteo cloud serve` can list the projects, open a project's sessions
  and memories, read one memory whole, and search titles, content and topic
  keys, all server-rendered with no new dependency. A user sees only the
  projects their grants reach: the admin role signs in but does not bypass a
  grant, and a project, memory or scoped search outside it is refused with 403.
  Lists are cut at published limits and say so on the page (#134).

- **The language menu says which translation a machine made.** Romanian was
  added machine-translated rather than written by somebody who speaks it, and the
  menu that offered it said nothing, which reads as a native translation. It now
  carries the mark beside its name, in Leteo's own language menu and in Sardi's;
  the memory language is left unmarked, because that setting is handed to a model
  rather than spoken by Leteo (#212).

- **`leteo --version` names the build, not only the release.** It prints the
  commit it was built from and the schema it supports — `leteo 0.3.0 (f712df8,
  schema 22)` — embedded by `build.rs` at compile time and degrading to the
  version and the schema alone when there is no git to read. `doctor` and the
  MCP `serverInfo` carry the same string, so a store that was refused can say
  which binary refused it (#208).

- **`leteo doctor --check <code>` runs only the check it names.** Asking about
  `busy_timeout` no longer pays for `PRAGMA integrity_check`, which is several
  seconds over a 96 MB store. The counts and pragmas are still gathered, and a
  check that did not run leaves its own aggregate absent rather than zeroed, so
  nothing reads as "the index holds nothing" when the index was never looked at
  (#204).

- **Catalan, Basque and Polish memories are stemmed in their own language.**
  `stemming::algorithm` now carries the three through `snowball_stemmers_rs`,
  so `migracions` finds a memory about `migració`. The setting already offered
  them; they got `porter` alone. The eight languages `rust-stemmers` already
  carried keep it, because the two implementations disagree on words they
  already index and the ratchet's floors are held to its stems. Each new
  language has an inflection set and a 1.000 floor (#183).

- **A migration leaves a restorable copy of the store it rewrote.** Opening a
  store stamped below the current schema now snapshots it first, to
  `leteo.db.pre-schema-N`, with `VACUUM INTO` — one consistent file with the WAL
  already folded in, so it is a way back from a one-way upgrade without copying
  `-wal` and `-shm` by hand. The newest three copies are kept. (#199)

### Changed

- **Sardi says the save reminder and nothing else by default.** Its voice was
  `all`, so a report line went into the conversation on every prompt, in every
  session, for everybody who never opened the settings. The default is now
  `reminders`: the line that does work still comes, and the reports wait for a
  file that asks for `all`. A settings file that names a voice keeps it (#212).

- **`doctor` weighs a finding by what it costs.** Each check carries an
  `error`, `warning` or `info` severity and `healthy` means no `error`, so a
  store whose semantic model is missing, or that holds a memory typed under a
  word no filter can name, is reported as degraded rather than broken — and one
  whose full-text index has gone empty is still an error. `leteo doctor` exits
  non-zero when an error exists and zero otherwise (#204).

- **Every tool reply carries its JSON as text again, at every protocol
  revision.** 0.3.0 made a reply on `2025-06-18` or later send one sentence —
  `Result in structuredContent.` — instead of repeating the answer, on the
  premise that a client negotiating that revision reads the structured half.
  The protocol says SHOULD, not MUST, and nothing had measured which clients
  feed `structuredContent` to their model, so a client that reads only
  `content` got nothing usable and a refusal lost its `error.code` and the
  `available_projects` and `recovery_token` the ambiguous-directory recovery
  flow reads. The JSON is now sent beside `structuredContent` at every
  revision, refusals included, until a client is measured that reads the
  structured half and not the text. The cost is the duplicate on the wire —
  about twice the bytes for a reply both blocks carry — and the "half of every
  reply, paid in an agent's context" the 0.3.0 note claimed was never measured
  as a token count (#112, #200).

### Fixed

- **A session recorded in a relative directory no longer matches every
  repository.** When the working directory could not be read, a save was filed
  under a project called `unknown` at `.`, and `.` was then resolved against
  whichever directory the reader stood in: the write gate offered `unknown` as
  an `ambiguous_project` candidate everywhere, the session-start hook could fold
  a project on its strength, and `projects consolidate` grouped by it. A relative
  recorded directory now matches nothing, an unreadable working directory
  refuses a write that relies on detection with the reason instead of filing
  it under `unknown` (a CLI save naming `--project` is still written, under no
  directory), and `create_session`,
  the replicated session upsert and spool replay store no directory rather than
  a relative one. Rows already holding `.` are neutralised without a migration;
  the stranded `unknown` project can be merged away with the existing project
  merge once upgraded (#259).

- **A memory typed under a word outside the eight is filed where a filter can
  reach it.** `normalize::kind` folds an unknown type onto `discovery`, the
  bucket the rest of the unclassifiable lands in, so a memory saved as
  `implementation` or `optimization` is no longer invisible to every typed
  search. A store written before the fold keeps the old words and `doctor`
  reports them as a warning until `--repair` folds them (#204).

- **`uninstall` stops deleting copies people made.** It matched the data
  directory by a bare `leteo.db` prefix, which is every hand-made
  `leteo.db.bak-before-migrate` and every `pre-schema` copy a migration left —
  the one file the upgrade notes asked for, deleted by the command that deletes
  the store. The store and its sidecars are now named exactly, and
  `uninstall --yes --keep-data` leaves the data directory itself. (#199)

- **A `nearest` answer no longer comes back as a page of unrelated memories.**
  The semantic stage merged its whole list beside a lexical answer that had
  reached `nearest` through one shared word, so a question the store could not
  answer filled every slot. It now merges only its best five. The benchmark
  carries 24 questions about things no memory in it is about and reports, per
  engine, how many come back with something, how many without a caveat, and the
  reply bytes; the mean on those questions fell from 10,900 to 5,493 bytes
  (#197).

## [0.3.0] - 2026-10-07

Search by meaning, by half-remembered words and through typos, stemming in the
language a memory was written in, a history of what an update replaced, a way to
merge duplicate memories, and a script that builds a checkout and installs it.
**`mem_update`, `mem_delete`, `mem_pin` and `mem_unpin` now require
`expected_project`**, so an agent that calls them without it is refused. Read this
before installing it beside an older Leteo: **0.3.0 migrates the store to schema version 22, and no
0.2.x binary can open it afterwards.** One that tries says so and refuses
(`this database is at schema version 22, but this build of Leteo understands
18`) rather than reading a store it does not know, so upgrade every agent's
Leteo together — `leteo setup <agent>` again repoints an agent at the new
binary. Leteo takes no copy before it migrates: if you may want 0.2.x back,
copy `leteo.db` together with its `-wal` and `-shm` files from the data
directory (`~/.leteo` unless `LETEO_DATA_DIR` moved it) before the first 0.3.0
run. That first open runs
migrations 19 to 22 once, stemming every memory and rebuilding the full-text
index. The semantic model is part of this tag, so `leteo model install` now
finds it for an install that arrived without one; at v0.2.1 there was nothing
to fetch.

### Added

- **`scripts/build-install.sh` and `build-install.ps1` build the checkout they sit in
  and install it.** They build with the pinned toolchain and `--locked`, install
  into Cargo's own root (`CARGO_INSTALL_ROOT`, then `CARGO_HOME`, then `~/.cargo`)
  and never the release installers' directory, install the model from
  `assets/model/` without a network, run `setup` again only for the agents that
  already have Leteo — asked of the binary through the `uninstall` preview, whose
  reply is refused unless it is one — and finish with `doctor`. Every failing
  step stops the run and names its command. `tools/build-install/check.sh` runs
  both behind stand-in `rustup`, `cargo` and `leteo` in CI, and refuses to run
  rather than delete the checkout when `mktemp` fails, which the first version of
  it did not (#188, #191).

- **Memories are stemmed in the language they were written in, Spanish first.**
  `porter` stays on every memory, and each one gains the Snowball stems of the
  language named by the memory-language setting, recorded on its row so the choice is
  fixed per memory; a question is stemmed for `porter` and for every language the store
  holds. `ejecuta` now finds `ejecutaron`, which `porter` cannot. Spanish,
  Portuguese, French, German, Italian, Romanian, Dutch and Swedish have a second
  stemmer; Catalan, Basque and Polish have official Snowball algorithms that the
  stemming crate lacks, Galician has none, and those four record their language and
  get `porter` alone. Each stemmer has a set of questions that reach their memory
  only through another inflection, scored on the strict pass alone, and a floor in
  the ratchet. Schema version 22; the migration stems existing rows in the setting's
  language and rebuilds the index. A tool writing to the database file directly must
  register both `leteo_stem` and `leteo_stem_language`, or its inserts and its edits
  of a memory's text are refused; deletes and edits of `project`, `type` or
  `tool_name` alone are not (#125).

- **The semantic search model is a file beside the binary, and `leteo model install`
  puts it there.** A 13 MB static embedding model, verified against SHA-256s the
  binary was built with before a byte of it is used, found through one ordered list
  (`LETEO_MODEL_DIR`, beside the executable, `../share/leteo/model`, the data
  directory). Every release archive and the Docker images carry it; every other
  install fetches it with `leteo model install` from the files committed at the
  tag of its own version (or
  `--from <directory>` with no network), which `leteo setup` runs, after the agent is
  configured, when it finds none. `leteo doctor` reports
  whether the model is verified, missing or wrong, and `leteo uninstall` removes it from
  every place on that list but `LETEO_MODEL_DIR`, a file only if it hashes to its pin, and
  without `--yes` it lists what it would remove. The binary grows by 50 KB for
  the installer and the crate stays about 1 MB (#124).

- **A search the words cannot answer now goes on to look by meaning.** A seventh
  stage, after the six lexical ones, finds a memory that shares no word with the
  question — a paraphrase, or the same question in another language — using a static
  embedding model read from a file beside the binary: no network at search time, no server, nothing leaves
  the machine. On an empty answer it speaks only above a cosine of 0.30; on the weakest
  lexical answer (`nearest`) it is merged in by rank. A result it adds is marked
  `semantic`, and the page says why once. On the engram-bench corpus overall MRR goes
  from .835 to .887 and empty answers from 9 to 3 with no kind falling; on a set of
  2,028 LLM-generated questions that do not use their target's words it adds .101 MRR
  [.088, .114]. The model is the file the entry above describes; a binary that cannot verify it
  searches by words only, and `doctor` says why. The model runtime adds 1.5 MB to
  the binary, which CI bounds; memory while it runs is in `search.md` §15. Turn it off with `"semantic_search": false`
  in `settings.json`. Basque is the weak language: the model was not trained on it
  (#124).

- **Romanian is the thirteenth interface language.** `interface` and
  `voice_language` accept `ro` (or `română`), and the screens, the Sardi voice lines
  and the subagent-capture headings are translated. The translation is
  machine-made with no native speaker having read it, so expect the odd awkward
  sentence (#13).

- **Command Code is the fifteenth agent `leteo setup` configures.** `leteo setup
  command-code` registers the MCP server in `~/.commandcode/mcp.json` in the entry
  shape Command Code reads (`transport: "stdio"`, `enabled: true`) and writes the
  instruction block to `~/.commandcode/AGENTS.md`; removal and `leteo uninstall`
  take both back out. There are no hooks yet: Command Code sends a different
  payload from the one `leteo hook` reads, so a session is not opened or
  closed for it automatically (#43).

- **An update no longer destroys the text it replaced.** A topic-key save and a
  content-changing `mem_update` overwrote title and body in place, so the memories
  an agent curated most were exactly the ones that lost their history. Each such
  write now keeps the superseded title and body in `observation_versions` (migration
  19), the newest twenty per memory; a metadata-only change, a deduplicated save
  and an insert keep nothing. `mem_get_observation` takes `include_history` and
  returns them whole, newest first; `mem_save` and `mem_update` answer
  `replaced_bytes`, with a `hint` naming that read when the new body is under half
  of what it replaced. Versions replicate and travel through `leteo export` and
  `import`. Restoring is an ordinary `mem_update` with the old text; there is no
  restore tool (#127).

- **`mem_consolidate` merges memories, and a superseded memory stops listing.**
  `mem_consolidate` and `leteo consolidate` insert one replacement and record a
  judged `supersedes` relation from it to each source, in one transaction: a source
  in another project, or one the store does not hold, refuses with nothing written,
  and the replacement is filed where the sources are. Before this a judged
  `supersedes` was only a caveat beside a memory that kept appearing everywhere;
  now its target is left out of search, the opening block, `mem_context` and
  every other listing. Nothing is deleted, so reversing the relation brings the
  memory back, and `mem_get_observation` still returns it by id with its caveat
  (#132).

### Changed

- **The opening block and `mem_context` had no size bound.** They were bounded by a
  memory count and by per-line lengths, which is not a byte bound: a store with long
  rows grew the block without a ceiling, and the block is paid on every session and
  after every compaction. Each `ContextSize` now carries a byte budget — 15,000,
  32,000 and 49,000 bytes for `slim`, `full` and `deep`, the measured block sizes
  rounded up — applied at an entry boundary and never mid-line. What it leaves out is
  counted: the block prints how many memories, prompts and sessions it dropped, and
  `mem_context` answers `memories_omitted`, `prompts_omitted` and `sessions_omitted`
  (#114).

- **Every tool reply carried its JSON twice; it now carries it once.** A reply with
  `structuredContent` also repeated the same JSON as text so that a client predating
  structured output still got an answer — half of every reply, paid in an agent's context.
  From protocol revision `2025-06-18`, which introduced the field, the text block is one
  sentence and the answer is in `structuredContent`; an older revision, and a version the
  server does not know, gets the full JSON as text exactly as before. Measured over the
  wire against a real store, a 20-result `mem_search` is 15,917 bytes received against
  32,097 — about half (#112).

- **`mem_stats` named projects and counted nothing; it now carries each project's
  counts, bounded.** The reply listed the projects holding a memory and stopped there,
  which answers "which projects are there" but not "which one is real" — the question
  after an `unknown_project` or an `ambiguous_project`. Each entry now carries the
  project's live-memory, session and prompt counts and its last activity, most recently
  written first, and the list is bounded by the store's own list ceiling with
  `projects_omitted` counting what it left out, so a bounded list is not read as the
  whole inventory. `mem_doctor`'s per-project detail carries the same last activity, so
  a project past the ceiling can still be asked for it (#129).

- **Search finds a word somebody half-remembered.** Search matched whole tokens and
  stems, so a fragment failed the strict pass exactly as an unknown word did and the
  widened retry answered by dropping it: on the benchmark's partial-word set
  (`pgxpo`, `storyb`, `telemetr`) MRR was 0.231. When the strict pass finds nothing
  each word is now retried as a prefix, and then as a substring of a title (`telemetr`
  is not the start of `OpenTelemetry`); both add no word and drop none, and mark their
  results `partial`. MRR on that set is 0.923, with no other kind moving by more than
  0.02. A trigram index would have done it for 18.5 MB on a 9.3 MB corpus and was
  declined; the title scan costs no bytes and about 3 ms on a 5,243-memory store
  (#123).

- **Search corrects a misspelled word, and says which.** A typo that stemming did not
  happen to collapse (`limitting` does; `conection` does not) found nothing. A word the
  index has never held is now replaced by the nearest word the unstemmed vocabulary does
  hold (one edit up to five characters, two above), all or none, and only when the
  uncorrected query found nothing. The reply names every correction: `mem_search`
  carries a `hint` and `leteo search` prints the same sentence on stderr. Typo MRR on
  the benchmark goes from 0.615 to 1.000 and overall from 0.784 to 0.835; a corrected
  query costs 20 to 30 ms on a 42,538-term store, and one that answers does not touch
  the vocabulary (#126).

- **A search query over 8,192 bytes is refused, naming the cap.** A pasted log was
  tokenised whole and the strict pass built one term per distinct word, so the cost of
  the input was paid before any stage could answer: 18.5 ms at 8 KiB, 132.5 ms at 64 KiB
  on a 4,000-memory corpus, close to linear in the terms. `mem_search` answers
  `query_too_long`, a code of its own beside `invalid_search`, and `leteo search` fails
  with `search query is N bytes, over the 8192-byte maximum` (#130).

- **A memory write names the project it expects the memory to be in.**
  `mem_update`, `mem_delete`, `mem_pin` and `mem_unpin` acted on a bare id, and ids are
  not private to a project: `mem_search` with `all_projects` hands out other projects'
  ids, so an agent in one repository could revise, pin or hard-delete another's memory
  by mistake. They now require `expected_project`, compared with the row's project
  inside the write transaction, and refuse with `project_mismatch` and nothing changed.
  Moving a memory takes both ends: `expected_project` is where it is, `project` where it
  goes. Reads, the CLI and the TUI are unchanged (#116).

### Fixed

- **The uninstall scripts no longer count a binary that cannot start as one that
  judged the model.** `uninstall.sh` treated any executable as having run, so a
  binary that failed to start left the model files behind and pointed at a report
  that was never printed; `uninstall.ps1` now agrees with it and retries the data
  files after a failed run, as the shell script already did. Both scripts decide
  that the binary started from the line `leteo uninstall: started`, which
  `uninstall --yes` now prints on stderr before it judges anything, and not from an
  exit code: under dash a binary of the wrong architecture is re-run as a script
  and exits 2, which the 126-or-127 rule counted as a run (#179).

- **Changing a remote split a project in two without a word.** The project is
  re-derived from `origin` on every call, so adding a remote to a repository named by
  its directory, renaming it or pointing it at a fork filed the next write under a new
  name while the memories already recorded stayed under the old one, and neither half
  could see the other. `mem_save` and `mem_session_start` now check what sessions this
  directory recorded, and when it was another project they answer `ambiguous_project`
  naming both sides and `leteo projects consolidate`; an explicit `project` still
  resolves it, and a directory whose sessions agree is not asked. The session-start
  hook cannot prompt, so it warns, and says when it could not look; passive capture
  continues under the recorded project. The extra read costs 235 µs at 500 sessions and
  2.24 ms at 5,000 (#131).

- **`setup` pinned a path `brew upgrade` deletes, and nothing noticed.** For a Homebrew
  install `setup` wrote `<prefix>/Cellar/leteo/<version>/bin/leteo` into every agent
  config; the upgrade removed that directory, every hook and the MCP server then failed,
  and a hook fails silently by design, so memory stopped being recorded with no signal.
  `setup` now writes the stable link when the binary sits under a versioned directory
  (Homebrew's Cellar, mise's and asdf's `installs/`) and the link exists, and `leteo
  doctor` reports an absolute binary path in any agent's MCP or hook configuration that
  no longer exists, as `missing_binaries` naming the file (#117).

- **Adopting an Engram v3 database re-armed what Engram had quarantined and could abort
  on a database that had synced.** `leteo import --from-engram` copied quarantined and
  superseded `sync_mutations` back in as pending; it folded two spellings of a project
  into two rows, not one; and it copied `sync_mutations` before the `sync_state` its
  foreign key points to, which failed the whole adoption. Quarantined and superseded
  mutations are now left out, the project fold merges, the order is fixed, and the
  report carries a `dropped` list naming every source table and column Leteo could not
  carry (the hard-delete tombstones among them) instead of skipping them silently
  (#118).

- **Claude Code cut the server instructions short.** It delivers the first 2,048
  characters of an MCP server's instructions and appends `… [truncated]`;
  Leteo's were 2,246, so every session lost the whole paragraph on summaries and the
  tail of the one on conflicts. The block is rewritten shorter with every rule still
  stated, and a test counts it against `SERVER_INSTRUCTIONS_LIMIT` in the unit the client
  counts (#111).

- **Opening a store could be aborted, or report a repair that did not land, when an
  adopted table's `id` was not an integer.** Migration 18's review-clock write-back
  addressed rows by `id`; on a table adopted with its own column definitions a TEXT
  primary key (`'007'`) never matched the bound integer, nothing updated, and the repair
  still counted itself. It addresses `rowid` now. The refusal for a database stamped
  with a pre-release schema version also named a remedy that cannot be reached (export
  and import into a fresh store); it says to set `PRAGMA user_version = 0` and reopen,
  or start fresh (#85).

- **A body over the storage bound was cut without saying so.** `mem_save` and
  `mem_update` stored at most `max_observation_length` bytes — 50,000 — and the
  tail was gone, while the reply said nothing: `content_truncated` describes the
  400-byte preview, so a body that came back whole but was stored short read as
  untruncated, and an agent that needed the whole thing had no way to know it
  had to split it. The same bound governs prompts, session summaries and a
  judgment's reason and evidence, and those tools report it too:
  `storage_truncation` on a save, update, prompt or session, and
  `reason_storage_truncation` and `evidence_storage_truncation` on a judgment,
  each carrying `original_bytes` and `stored_bytes`, a field of its own rather
  than the preview flag beside it, and absent when nothing was cut. The length is
  taken after redaction, so a body whose bulk was a `<private>` span is stored
  whole and reports nothing (#122).

- **Ending a session wiped a summary it had already been given.** The
  `SessionStop` hook closes every session with no summary, and `end_session`
  assigned it — so a summary written with `mem_session_end` was gone the moment
  the session closed, and the opening block and `mem_timeline` lost what the
  conversation had been for. The end now keeps the summary it finds, the way the
  replicated path already did.

- **A save could be filed under a session that had already ended.** A write
  naming any existing session was accepted, so a memory could land in a
  conversation that had closed and never appear beside its own prompts.
  `mem_save`, `mem_save_prompt`, `mem_session_summary` and `mem_capture_passive`
  now answer `session_already_ended`, and `leteo save --session` refuses the same
  way; a save that names no session keeps landing in the project's manual
  session, and the unnamed path never reads the ended state, so it cannot be
  refused (#121).

- **The OpenCode plugin did not load on OpenCode 2.x.** It exported only the 1.x
  `server` factory, so on 2.x — which calls `setup` — the plugin never loaded and
  no lifecycle capture happened, silently. One file now serves both: the default
  export carries `server` and `setup` over the same four handlers, and the hook
  path spawns through `node:child_process` instead of `Bun.spawn`, because 2.x
  runs plugins on Node where `Bun` is undefined. `plugin/opencode/leteo.test.ts`
  holds the plugin to each major's contract, and a Rust guard reads the file so
  `cargo test` watches it too (#120).

- **An Engram 0.2.0 backup could not be imported.** Engram moved its direct backup
  to `0.2.0` in v3.0.0, and `leteo import` refused it with "unsupported export
  format 0.2.0; this build reads 0.1.0" — so the JSON fallback for migrating
  another machine or a backup only worked for Engram ≤ 1.20. With the version
  check passed it then failed on the relation's missing `id`, which 0.2.0 stopped
  writing. Both shapes are read now: relations deserialize without an `id` and are
  keyed by `sync_id` on insert, and the `prompt_tombstones` 0.2.0 added land in
  `prompt_deletions` so a deleted prompt is not resurrected. Leteo still writes
  `0.1.0`; an unknown future version is still refused by name (#119).

- **The preview tools promised characters but cut bytes.** Every description that
  previews a body said "a 400-character preview", and the cut is `PREVIEW_BYTES = 400`
  bytes — so in Spanish, CJK or emoji an agent was shown less than it was told. The unit
  is now bytes everywhere it is published: the eight descriptions say "a 400-byte
  preview", `PREVIEW_BYTES` says bytes in its own doc comment, and `mcp-tools.md` §3
  states it. The cut does not move; the promise now matches the limit applied (#113).

- **Two tools described their `relation` argument as a "Verdict", and an agent read the
  description as the key name.** `mem_judge` and `mem_compare` publish the argument as
  `relation`, but the `description` beside it began with the word "Verdict" — the slot where a
  key name would sit. On 2026-09-27 an agent that had just listed the tool called `mem_judge`
  with `verdict: "related"`; the call was refused before it ran (`relation: Missing key`) and
  retried correctly eight seconds later. The two descriptions now say how the two observations
  relate; the key and the six accepted verbs are unchanged (#109).

- **The Windows binary needed a runtime the docs said it did not.** Every
  Windows release so far linked the MSVC CRT and UCRT dynamically, so
  `leteo.exe` imported `VCRUNTIME140.dll` and ten `api-ms-win-crt-*.dll`
  forwarders — a moderator reviewing the winget manifest caught it
  (microsoft/winget-pkgs#416516) and asked for `Microsoft.VCRedist.2015+.x64`
  to be declared as a dependency, which is exactly what the README's "nothing
  else to install first" promise said would not be needed. `.cargo/config.toml`
  now links the CRT and UCRT into the binary instead
  (`target-feature=+crt-static`); read back with `dumpbin /IMPORTS`, the
  binary carries neither `VCRUNTIME140.dll` nor any `api-ms-win-crt-*.dll`
  afterward. The cost is 209,408 bytes on a 19 MB executable (+1.1%);
  measured interleaving 30 runs of each binary, neither `leteo --version` nor
  `leteo hook user-prompt-submit` came out measurably slower. The README now states
  the floor, Windows 10 or later, and the release job reads the import table of the
  `leteo.exe` it is about to archive (`scripts/check-windows-imports.ps1`) and fails on
  any DLL outside an allow-list, so a `RUSTFLAGS` that replaces the link flags goes red
  instead of shipping (#100, #102, #103).

### Internal

- Search quality and reply size are now held in CI: a ratchet loads a synthetic
  170-memory corpus into a fresh store, asks 117 questions and fails when any kind's MRR
  or the bytes of a search or `mem_context` cross `tools/engram-bench/floors.json`
  (#128). A pin-and-recency rerank in Engram's style was measured through the shipped
  query and declined: neutral on bodies, and on title-shaped questions MRR fell from
  0.8666 to 0.8042 (#115). The measured comparison with Engram is generated by one
  command into `docs/comparison-with-engram.md` (#135).
- CI can be dispatched against a head a publication just pushed, and skips the suite
  when a pull-request run already covers that head (#57). The setup tests no longer
  write to the real agent configuration when `CLAUDE_CONFIG_DIR` or another root is
  set: a `cargo test` run inside Claude Code had overwritten the author's
  `settings.json` (#136).

## [0.2.1] - 2026-09-02

Everything 0.2.0 was is already in it. This is the release that reaches the
registries a tag was never able to reach, and there is no reason to install it
over 0.2.0 unless you want it from the MCP registry.

One thing to hold while reading the three entries below, because they describe
one incident from three angles and the route between them is easy to miss.
Tagging 0.2.0 published the binaries, the container images and the GitHub
release, and reached **no registry at all**. crates.io and npm did get 0.2.0
afterwards, from the same workflow dispatched by hand — which is what
`workflow_dispatch` was put there for, and what produced the timings and the
error messages quoted below. The MCP registry never got it, for the third
reason, which no dispatch could work around.

### Fixed

- **A tag could not publish to any registry, and no tag had ever tried.** All
  three registries authenticate by OIDC and each trusts a workflow *by
  filename*. `release.yml` reached them through `uses:
  ./.github/workflows/publish-registries.yml`, and in a reusable-workflow call
  the token names the **caller** — so the job presented itself as `release.yml`
  and crates.io refused it: *"The Trusted Publishing config for repository
  `asanabrial/leteo` does not match the workflow filename `release.yml` in the
  JWT. Expected workflow filenames: `publish-registries.yml`."* npm said the
  same thing as a `404` on the `PUT`, which is its phrasing for unauthorised.

  Nothing had noticed because nothing had run: 0.1.2 shipped before those jobs
  existed, so **0.2.0 was the first tag to use this path and it failed the
  first time**, at the end of a ten-minute release with five binaries and two
  container images already built. `release.yml` now dispatches that workflow
  instead of calling it, so the token carries the filename the registries
  trust, and the `workflow_call` trigger is gone rather than left beside it —
  a `uses:` added back would fail identically and take another release to find
  out. What this costs is that the registry results live in a second run: a tag
  that published and a tag whose registries failed look the same in the release
  run.

- **The MCP registry submission raced the crates.io publish it depends on.**
  All three jobs started together, and the registry validates every package
  `server.json` offers before accepting a submission. It asked crates.io for
  0.2.0 at 21:13:10; crates.io recorded the crate at 21:14:44. It failed by
  ninety-four seconds, naming a precondition that became true before anybody
  could have checked — the worst kind of red, because re-running it passes and
  teaches nothing. It now waits for both crates.io and npm.

- **The npm package did not name the MCP server, so the registry refused it.**
  The registry reads `mcpName` out of the *published* tarball and rejects a
  submission without it. `server.json` has named the server
  `io.github.asanabrial/leteo` since the beginning; `npm/package.json` had
  never carried the field, and nothing in any language compared the two. That
  is why 0.2.0 reached crates.io, npm, GHCR and the GitHub release while the
  MCP registry got nothing — and why it needed this release rather than a
  repair: npm does not let a published version be replaced, so the field could
  only ship in the next one. A guard now holds the two names together.

## [0.2.0] - 2026-09-01

### Added

- **DeepSeek Harness joins the agents `leteo setup` configures.** Its global
  patch layer, `$DSH_HOME/cordis.patch.yml` (`~/.dsh` by default, moved by the
  `DSH_HOME` environment variable), is the file every profile reads to compose
  its session, so an `mcp-client` row inserted there reaches the web GUI and
  every other profile at once — the tools surface to the model as
  `mcp__leteo__<tool>`. The protocol block goes to `$DSH_HOME/AGENTS.md`, the
  fixed user-global instruction file loaded into every session. The harness
  has no auto-discovered hook settings file, so — like Cursor, Gemini CLI and
  the other MCP-only clients — it takes no lifecycle hooks.

- **ZCode joins the agents `leteo setup` configures.** Its servers land under
  the nested `mcp.servers` of `~/.zcode/cli/config.json` — one JSON document
  the client also holds its providers, plugin state and hooks in, so every
  edit splices in place and leaves the rest exactly as it arrived. Its
  instruction block goes to `~/.zcode/AGENTS.md`. Three of the five lifecycle
  hooks register: ZCode supports neither `SubagentStop` nor `SessionEnd`, and
  `session-stop` stays unregistered rather than moving onto `Stop`, which
  fires at the end of every reply. Config-file hooks start switched off in
  that client, so setup turns the runner on — or refuses when somebody has
  deliberately turned it off.

- **A plugin bundle for ZCode, in the marketplace this repository already
  serves.** `plugin/zcode` carries the MCP entry, the memory skill, and the
  three lifecycle hooks that client fires, under `.zcode-plugin/plugin.json` —
  the manifest path ZCode looks for first. It is listed in
  `.claude-plugin/marketplace.json` beside the Claude Code bundle, which is the
  file both clients read at the repository root. Adding it is a desktop action —
  **Settings → Plugins → Create → Add marketplace** — because that client's CLI
  lists and enables installed plugins and does not add marketplaces at all.

  Measured against a real ZCode 0.16.5 rather than taken from its documentation:
  the client already carries `anthropics/claude-plugins-official` registered as a
  `github` source in `~/.zcode/cli/plugins/known_marketplaces.json` and caches a
  repo whose manifest is at `.claude-plugin/marketplace.json`, which is the path
  Leteo publishes; its plugin cache is laid out
  `plugins/cache/<marketplace>/<plugin>/<version>`, which is what
  `AgentAdapter::plugin_cache_root` already assumed; and the installed binary
  carries `userConfigDirSegments`, `.zcode/cli` and `hooks.enabled` as strings,
  which is the config path and the runner switch the adapter was written against.

  The bundle is not just a second way to install the same thing. ZCode runs
  configuration-file hooks only while `hooks.enabled` is true, and that switch
  starts off and belongs to the person, not to Leteo; enabling a plugin is what
  enables the plugin's hooks. On a machine where that switch is somebody else's
  decision, this is the route that works.

  Which events a bundle carries is now taken from the agent rather than from the
  global list. Held against `HOOK_EVENTS`, ZCode's honest three-event bundle
  would have failed and a bundle carrying two events its client cannot fire
  would have passed. The marketplace guard grew the same way: it checked
  `plugins[0]` and would have watched the Claude bundle forever and never the one
  added beside it, and the skills were compared as a pair, which stops being one
  rule the moment a third arrives.

- **A Homebrew tap and a Scoop bucket, for people who keep their tools in one
  place.** `brew tap asanabrial/leteo && brew install leteo` covers macOS and
  Linux on both architectures, and `scoop bucket add leteo
  https://github.com/asanabrial/scoop-leteo` covers Windows. Both went up after
  `0.1.2` shipped and nothing in this repository mentioned either, which is the
  same shape the plugin marketplace had: a route that works and appears in no
  file anybody reads.

  One paragraph had to be narrowed rather than added to. It said the binary
  lands in `~/.local/bin` or `%LOCALAPPDATA%\leteo\bin`, which is true of the
  install scripts and not of a package manager — Homebrew and Scoop put it
  where they put everything else, and a reader who installed that way would
  have gone looking in a directory with nothing in it.

### Changed

- **The install scripts moved to `scripts/`.** The one-liners are now
  `raw.githubusercontent.com/asanabrial/leteo/main/scripts/install.sh` and
  `.../scripts/install.ps1`. The old paths 404: they pointed at `main`, so
  moving the files broke them the moment this landed, and there is no redirect
  a raw URL can leave behind. Every copy this project controls was updated in
  the same commit; a copy somebody else made was not, which is the whole cost
  and the reason to do it at four days old rather than at four months.

  Release archives already downloaded are unaffected — they carry the binary
  and the uninstaller inside them and fetch nothing.

- **The four Docker files moved to `docker/`.** `Dockerfile`, `Dockerfile.mcp`,
  `docker-compose.yml` and `.env.example` sat at the repository root in `0.1.2`
  and are one directory between them now, which is three fewer rows GitHub
  lists before it reaches the README. A checkout of `0.1.2` that ran
  `docker compose up` or `docker build -f Dockerfile .` from the root has to
  name the directory: `docker compose -f docker/docker-compose.yml up`, or
  `-f docker/Dockerfile`.

  This is the same move the install scripts made, and it is written down for
  the same reason — except that nothing 404s here, because these files are read
  from a clone rather than fetched from a URL. `.dockerignore` now excludes
  `docker` rather than the two files by name; the build still finds a
  Dockerfile it has been told to ignore, measured with `-f docker/Dockerfile.mcp`
  getting past every `COPY` and into compiling Leteo, and CI builds both images
  on every pull request.

  The published image tags are unchanged, and a person who installs the binary
  rather than running the cloud service is unaffected.

### Fixed

- **A ZCode client got no tools at all, because `tools/list` answered without
  the two fields the revision it had just negotiated requires.** Leteo
  negotiates protocol revision `2026-07-28` when a client asks for it, and then
  answered `tools/list` without `ttlMs` and `cacheScope`, which SEP-2549 makes
  that revision require. The `#[tool_handler]` macro filled both with `None`
  and rmcp never serialises a `None`, so a client speaking the revision rmcp
  itself had offered received a result it was obliged to reject — rmcp strips
  `resultType` for peers on older revisions but fills nothing for newer ones,
  so the compatibility was built backwards only. ZCode 0.16.5 ended every
  connection as failed and retried in a loop: zero tools, no explanation, and
  the lifecycle hooks still working, so nothing looked broken from outside.

  The macro leaves no seam for the result it builds, so `list_tools`,
  `call_tool` and `get_tool` are written out with the bodies it generated, and
  `list_tools` is the one that gains the two setters — `ttlMs` of five minutes,
  which bounds how long a client would serve a list from a process that has
  since restarted, and `cacheScope` of `public`, because the list depends on the
  `--tools` flag and never on who is asking. It sets them for a session that
  negotiated `2026-07-28` **or newer**: the gate is a comparison rather than an
  equality, so a revision rmcp has not shipped yet keeps them. At `2025-11-25`,
  `2025-06-18`, `2025-03-26` and `2024-11-05` they stay absent rather than
  published for older clients to tolerate or choke on, and `2025-11-25` is the
  answer an unrecognised revision gets, so a client that asks for nothing in
  particular is one of those four. The tool list itself is byte-identical to
  before.

- **Two sections of the opening block printed a title no limit applied to.**
  `hooks.md` §5 promises every line of that block is cut at a bound somebody
  measured, and it names the title bound first. Of the four places `recall.rs`
  prints a title, the two that also print a *content* preview — `### Pinned` and
  `### Recent Observations` — passed it through `one_line`, which folds
  whitespace and cuts nothing. The only ceiling left on them was
  `max_observation_length`: fifty thousand bytes, the cap a memory's *body*
  gets, in the section that is the bulk of the block, against five session lines
  cut at 320 and ten prompt lines cut at 200. One title could have outweighed
  every bounded line in the block put together. Both now cut at `TITLE_CHARS`,
  the 140 that was already there and already imported, rather than a second
  constant for the same idea.

  Nothing had fallen in yet, and the entry says so rather than implying a
  rescue: measured over a copy of a real store, the two sections rendered 89
  title lines across 27 projects and the longest was 132 characters, so the same
  81 blocks come out byte-identical, 300,328 B either way. It is not a vacuous
  bound either — 93 of that store's 4,756 live titles are already past 140 and
  would be cut today had they landed in either window. It reaches every surface
  that renders the block: the session-start and compaction hooks, `leteo
  context`, and the `mem_context` tool.

  Three genuinely unbounded sites remain in `src/hooks/context.rs`. They are a
  separate issue and are not fixed here.

- **Setting up DeepSeek Harness broke the harness on the shape the harness
  itself ships.** Its patch layer is a top-level YAML array, and a profile's
  arrives holding `[]`. Leteo appended its row as a block entry without asking
  what notation was already there, so `[]` ended up followed by `- insert:` —
  two nodes in one document, which no parser accepts. Measured end to end
  against a real `~/.dsh`: `leteo setup deepseek-harness` produced a file that
  fails at the first row with "expected `<document start>`", and because that
  layer is what every profile composes, what it broke was every session on the
  machine rather than Leteo's server.

  `[]` is the same array said the other way, so it is replaced when the first
  row arrives and put back when the last one leaves — a file of comments alone
  would be `null`, not the empty array its header says it is. A flow array with
  entries in it, or a top-level mapping, is now refused with the file named
  rather than corrupted: merging into either needs the YAML parser this crate
  deliberately does not carry.

- **A ZCode config holding an event in a shape Leteo does not write crashed
  setup instead of refusing it.** `hooks.events.<Event>` was read straight into
  `as_array_mut().expect(…)`, so an event the client had emptied to `null` — or
  a hand-edited one holding an object — panicked, with a message claiming the
  array had just been created. The nested renderer now makes the same two passes
  the flat one always has: a `null` is normalised, anything else is refused with
  the key and the file named, and an event Leteo never writes is left in
  whatever shape it was found.

- **Uninstalling DeepSeek Harness emptied a patch file it could not decode.**
  `remove_dsh_server` took the bytes with `unwrap_or_default`, so one Latin-1
  accent in `cordis.patch.yml` turned the whole document into the empty string
  and the file went back to disk with nothing in it, reported as an ordinary
  update. That file is the machine-global layer every profile composes. It is
  now refused the way the install side of the same file always refused.

- **A ZCode hook runner somebody had switched off cost that agent its memory
  entirely.** The refusal lands before the MCP server is written, which is right
  for a typed `leteo setup zcode --hooks` and wrong for the wizard, where a
  ticked ZCode came out with nothing configured at all over the hooks half of an
  answer. The wizard now asks first and installs everything the refusal was not
  about — the same shape it already used for a plugin bundle that registers the
  hooks itself.

- **`doctor` called ZCode's hooks healthy when nothing could run them.** Setup
  turns the runner on, so the case that reaches a machine is somebody turning it
  back off afterwards: every hook stops and the file still names every command,
  which is all the check read. It now reports the switch, the way Codex's
  untrusted hooks are reported beside it.

- **The `--tools` argument went unquoted into the DeepSeek Harness patch.** It is
  free text off the command line, and an apostrophe closed the YAML scalar early
  and wrote a row the harness cannot parse — which costs every profile its
  session, not just Leteo's server. It goes through the same quoter as the
  executable path.

- **A malformed servers key no longer says which file it is in.** Walking a key
  *path* rather than one key dropped both the file and the key that actually
  failed from the message; with fourteen configuration files, "mcp.servers must
  contain a JSON object" named neither the document to open nor which of the two
  keys held something else. Both are back.

- **`ghcr.io/asanabrial/leteo` publishes `linux/arm64` as well as
  `linux/amd64`.** It shipped amd64 alone, because the build step passed no
  platform and took whatever the runner was, so an Apple Silicon Mac or an
  arm64 server ran the cloud image under emulation — working, and slower for
  no reason visible from the outside. Each architecture is now built on a
  runner of its own and joined under the release tags, rather than emulated —
  as the release binaries for that architecture already are.

  The published tag names are unchanged, and nothing about the local product
  changes — it ships as a plain binary and uses no image.

- **Two implementations of "six calendar months" disagreed at a month's end, and
  the schema version moves to 18.** `rules::review_after` clamps a day the target
  month does not have onto that month's last day; the baseline migration's
  `datetime(created_at, '+6 months')` rolled it forward instead, so a decision
  written on 31 August was due 3 March rather than 28 February. Measured over
  2026-2029, the two disagree on 27 days for the six-month window, 19 for three
  months and 1 for twelve. Migration 18 — the first after the baseline, and
  written in Rust because expressing the repair in SQL would mean answering in
  the dialect that caused it — puts the affected clocks back, touching only a row
  whose clock is exactly what the baseline's arithmetic would have produced, so a
  clock somebody set by reviewing is left alone.

  `SCHEMA_VERSION` goes from 1 to 18 because every number from 2 to 17 has been
  stamped on a real file by the pre-release numbering. Those stamps are still
  refused, now with a message of their own: what they did lives in the folded
  baseline, which runs only for an unstamped database, so bringing one forward
  would give it this migration and none of the pre-release migrations above its
  own number — a store stamped 8 holds what 0002 through 0008 did and has never
  seen 0009 through 0017.

## [0.1.2] - 2026-08-12

The npm wrapper published in 0.1.1 could not download on Linux. This is that,
and the hole it uncovered next to it.

### Fixed

- **`npx @asanabrial/leteo` downloads.** The wrapper used `fetch`, which
  answered `UND_ERR_SOCKET` on the 7.9 MB archive three attempts out of three
  in a container where `curl` fetched it with a 200 every time. It succeeded
  from Windows and failed from Linux against the same URL in the same minute,
  which is how it was published without anybody noticing: every check before
  publishing ran on the machine where it works. It uses `node:https` now,
  following GitHub's redirect by hand, with three retries and a timeout for the
  failures that really are ordinary — and the error carries the cause, since
  Node reports these as the bare words "fetch failed".
- **`leteo setup` refuses a binary npm is holding.** It writes the path of the
  running binary into an agent's configuration, and through the wrapper that
  path is inside npm's cache — deleted by `npm cache clean` or by the next
  version. The MCP server would stop starting and all five hooks would fail
  without saying so, which is what hooks do here by design. It now refuses
  while somebody is there to read it, and names the `npx` configuration to use
  instead.

## [0.1.1] - 2026-08-12

A distribution release. The binary does what 0.1.0's did; what changed is who
can run it and how many ways there are to get it.

### Fixed

- **The Linux builds run on Debian 12 and Ubuntu 22.04 again.** They were built
  on `ubuntu-latest`, which became 24.04, and a glibc binary runs on the version
  it was built against or newer and never older — so 0.1.0 asked for GLIBC 2.38
  and would not start on current Debian stable, after downloading and extracting
  perfectly. Both Linux targets now pin `ubuntu-22.04`, and the result asks for
  no more than GLIBC 2.34: checked running on Rocky Linux 9, Ubuntu 22.04 and
  Debian 12. The floor is written in the workflow and in the README rather than
  inherited from a label that moves.
- **The Codex plugin bundle registers the hooks the installer writes.** Its
  `session-start` matched an extra `resume`, so a resumed session was handed an
  opening block it already had, and its `SubagentStop` carried a matcher the
  installer leaves empty. The guard that holds the bundles to `HOOK_EVENTS`
  compared events and timeouts but not matchers — which is the whole of what
  separates `session-start` from `post-compaction` — and now compares those too.

### Added

- **`npx leteo mcp`.** A zero-dependency npm wrapper that fetches the release
  binary for your platform, checks it against the published `SHA256SUMS`, and
  hands it every argument. It is a way in rather than the way to run Leteo: a
  binary on your `PATH` starts without a download.
- **The plugin marketplace is documented.** `/plugin marketplace add
  asanabrial/leteo` has worked since before 0.1.0 and appeared in no file in
  the repository. The Claude Code and Codex bundles now have READMEs, including
  the two things that bite: installing the plugin *and* running `leteo setup
  --hooks` registers every event twice, and Codex does not fire hooks until the
  directory is trusted.

### Internal

- The version is written in six manifests and a guard now holds all six to the
  crate's, because none of them can see the others and a plugin manifest left
  behind means installed plugins never offer an update.
- A test that asserted two racing writers never see `DatabaseBusy` was
  measuring the runner rather than the store, and failed in CI on a commit that
  touched no Rust. It arranges the contention now instead of fighting for it.

## [0.1.0] - 2026-08-09

First release. Everything below is what this version does, rather than what
changed on the way to it — there is no earlier version to have changed from.

### What Leteo is

One Rust binary over one SQLite database, bundled, with FTS5 for search. A
coding agent saves what was decided, fixed and learned; Leteo keeps it across
sessions, compactions and machines, and hands it back when it is relevant.
Nothing leaves the machine unless cloud replication is turned on for a named
project.

### The store

- Memories carry a type from a fixed vocabulary of eight, a project, a scope,
  an optional topic key, and — for the three types that go stale — a date to be
  reread. Common synonyms fold on the way in *and* on the way out, so a memory
  saved as `bug` is stored as `bugfix` and a search for `bug` still finds it.
- A `decision` asks to be reread after six calendar months, a `policy` after
  twelve, a `preference` after three. The date is counted from the memory's own
  date, so a memory replicated five months late is due one month from now on
  both machines rather than six months from now on one of them.
- A topic key is how a subject evolves in place: a later save under the same key
  revises the memory instead of adding a second one beside it.
- Two memories can be said to relate — `related`, `compatible`, `scoped`,
  `conflicts_with`, `supersedes`, `not_conflict`. A memory a later one overturned
  is handed back saying so, on every surface that hands memories back.
- Sessions and the prompts that produced the memories are stored too, so a
  memory keeps the question it answered.
- Deletion is soft by default and hard on request. Both are journalled.

### Search

- Full-text with weighted BM25 over title, body and identifiers.
- A question is answered by matching every word first; if that finds nothing,
  the same question is asked again matching any of them. Over 200 questions
  against a 2,643-memory store, strict matching returned nothing for 4% of short
  questions and 12% of long ones, and the widened retry found the memory every
  time, at rank one every time. A widened answer is marked `partial` and says so,
  because "these matched some of your words" is a weaker claim than "these
  matched your question".
- An answer cut short by the server's own maximum says that, rather than looking
  like an exhausted list.

### Surfaces

- **MCP server** over stdio: twenty-two typed tools, each answering with
  `structuredContent` against a declared output schema, and each refusing a field
  it does not take. `--tools=agent|admin|all|<tool>` (or `LETEO_TOOLS`) chooses
  which are exposed; `--project` (or `LETEO_PROJECT`) sets a process-level
  project. Every reply carries which project it used and which authority chose
  it. Bodies are bounded at 400 bytes and say when they were cut;
  `mem_get_observation` is the one tool that promises a memory whole.
- **Lifecycle hooks**: `leteo hook <event>` handles session start, context
  compaction recovery, prompt capture, subagent capture and session stop,
  directly against SQLite. No shell, no HTTP server, no port, no `curl`, no
  `jq` — so they behave the same on Windows as anywhere else.
- **Command line**: saving, searching, timelines, context, projects, conflicts,
  deletion, import and export, diagnostics, replication.
- **Terminal UI**: a dashboard whose three lists narrow together as you type,
  paged rather than truncated, with counts that say what is on screen against
  what the store holds. Deleting asks first, names the target, and counts what
  goes with it.

### What a session opens with, and what it closes with

- A session opening hands the agent an index of what the project already knows —
  recent memories as previews, pinned ones, recent sessions and prompts — sized
  by a setting rather than by a constant.
- It also hands over the pairs still waiting on a verdict, oldest first, with
  the id each needs to be ruled on. Judging them is Leteo's own bookkeeping: the
  agent settles every verdict itself and never puts one to the user. Pairs that
  no call could ever settle — a memory deleted outright, or two ends that ended
  up in different projects — are counted and named rather than offered as work.
- Memories whose reread date has come round are counted, and `mem_review` hands
  them over.
- A prompt may be met with a memory that fits it. It speaks about four times in
  five and hedges when it does, because a hint that is sometimes wrong has to
  read like a hint.
- A quiet project is reminded to save, on a clock that stays civil.
- Sardi is the voice all of that is said in, in twelve languages.

### Projects

- The project is detected from the session, a process override, a `.leteo`
  config, the git remote, the git root, a single child repository, or the
  directory name — in that order, and every answer says which one it came from.
- A directory holding several repositories is ambiguous, and a write into one is
  refused with the candidates, a short-lived recovery token, and instructions to
  ask the user which they meant. An agent cannot invent a project or quietly pick
  one.
- A scan for sibling repositories that runs out of time says so rather than
  falling through to a confident guess.
- Projects can be listed, consolidated, merged and pruned.

### Replication

- Opt-in PostgreSQL cloud replication, per enrolled project, journalled one
  mutation at a time and applied exactly once. Your machine is the client; the
  cloud never connects back. This is the only replication there is — the journal
  is written against a named target and the codec that forms its chunks is
  transport-agnostic, but no command syncs one machine to another directly.
  Moving memories without the cloud is `export` and `import`, which carry the
  whole store rather than a delta.
- Background replication during `leteo serve` and `leteo mcp`, on its own
  connection.
- `leteo cloud admin` covers administrator bootstrap, managed tokens, project
  grants, service-wide pausing and database health.
- A relation whose two memories have not arrived yet is deferred and retried a
  bounded number of times, then retired as dead rather than retried forever.

### Setup and distribution

- Setup adapters for twelve MCP-capable clients, installing the server and,
  where the client supports them, the lifecycle hooks — idempotently, leaving
  the rest of the configuration file alone.
- Installable plugin bundles for Claude Code and Codex, and an OpenCode plugin,
  each registering the server, the hooks and a memory skill.
- `leteo setup` with no agent walks through setup when a terminal is attached
  and prints the machine-readable list when it is not.
- `leteo uninstall --yes` removes Leteo from all twelve agents and then from the
  machine. On Windows it registers itself so it appears in Installed apps.
- `install.sh` and `install.ps1` fetch the release for the running machine and
  verify it against the published SHA-256 sums, refusing anything that does not
  match. Releases build five targets: x86-64 Linux, Windows and macOS, and
  arm64 Linux and macOS.
- A Dockerfile, a Compose stack and a tagged release image for the cloud
  service.

### Coming from Engram

Leteo is a reimplementation of Engram. The attribution in `NOTICE` and `LICENSE`
stays: it is an MIT requirement, not a courtesy.

- `leteo import --from-engram` takes an existing installation's memories over,
  defaulting to `~/.engram/engram.db`. It copies the database with
  `VACUUM INTO`, so a running Engram's most recent memories come across and its
  own file is never written to. `--dry-run` reports what it would take. Adopting
  over a store that already holds memories is refused rather than merged.
  Copying beats exporting here: Engram's JSON carries sessions, observations and
  prompts but not the relation verdicts, so an export-and-import migration would
  silently drop every conflict judgement.
- Data compatibility is verified in both directions against upstream commit
  `763a6ba` built from source: a JSON export from either tool imports into the
  other, and either binary opens, reads, searches and writes the other's SQLite
  database. What does not cross is what only one side models — an Engram build
  ignores a memory's link to the prompt that produced it, and Leteo columns
  unknown to Engram simply go unread.
- Drop-in CLI compatibility is *not* promised, and the two differ in places:
  `leteo export --output FILE` takes a flag where `engram export FILE` takes a
  positional argument, and Leteo's cloud dashboard serves two routes where
  Engram's serves about thirty.
- The local tables are named for what Leteo stores rather than for what Engram
  called them: `user_prompts` is `prompts`, `cloud_upgrade_state` is
  `sync_upgrade_state`, `sync_apply_deferred` is `sync_deferred_mutations`, and
  `prompt_tombstones` is `prompt_deletions`.

### Schema

The local schema is versioned with `PRAGMA user_version`, and a database stamped
above what the running build understands is refused rather than written to by a
binary that cannot know what changed. This release ships one version: the
baseline under `migrations/0001_*`. Later changes live one per file and are
applied by number. A database carrying no version — an Engram store, or an early
one of either — is adopted by inspection rather than by replaying a history it
never had.

### Security

- MCP agents cannot invent or silently pick a project for a memory write.
- Imported sync chunks are bounded, so a hostile archive cannot exhaust memory
  through decompression.
- Cloud responses are bounded as they stream in, rather than after the whole
  body is buffered.
- Cloud startup requires explicit authentication and a dashboard signing secret,
  and legacy tokens require a project allowlist.
- Dashboard sessions revalidate managed-token revocation and the current admin
  role, and the session cookie is marked `Secure` unless the request is plainly
  local.
- Cloud clients reject plaintext HTTP except for localhost and loopback.
- Internal PostgreSQL errors are logged, not returned to clients.
- Cross-tenant isolation is covered end to end against a real PostgreSQL: a
  principal cannot push into, pull from, or read the manifest of a project it was
  not granted, and cannot widen its own grant to the wildcard.

### Known limits

- Retrieval is lexical. Measured, the gap that embeddings would close is real:
  questions phrased like a memory's title reach MRR ~0.96 and questions phrased
  like its body ~0.80, and no reweighting of the lexical fields moves that — the
  same idea in different words is a semantic problem. The columns
  (`observations.embedding`, `embedding_model`, `embedding_created_at`) are
  carried and empty.
- Three further columns are carried and never written:
  `observations.expires_at`, and `memory_relations.superseded_at` /
  `superseded_by_relation_id`. `review_after` covers what expiry would have, so
  they are inherited shape rather than an unfinished feature. Left in place
  because dropping a column is a migration that gains nothing.
- The MCP stdio transport silently discards a message it cannot parse — JSON
  that is truncated, not JSON at all, or nested deeper than serde's 128-level
  limit — instead of answering with a JSON-RPC parse error. The session stays
  healthy and every later request is served normally, but a client that sent an
  `id` waits for its own timeout. The behaviour is in the `rmcp` transport
  rather than in Leteo.
- PostgreSQL integration tests require an isolated `TEST_DATABASE_URL` and are
  ignored when it is absent.
- A pending pair whose two memories end up in different projects can no longer
  be judged. New ones are retired when the move happens; any left from before
  are reported by `leteo conflicts list --status pending` and are not offered as
  work.
