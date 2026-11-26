# Changelog

All notable changes to Leteo are documented in this file.

## [Unreleased]

### Added

- **A hook says where its time went.** `leteo hook <event> --verbose` prints the
  milliseconds each phase took — opening the store, detecting the project,
  building the opening block, the event itself and the spool replay — on
  standard error, where a hook log reads it and the agent's own context does
  not. Without the flag the answer is byte for byte what it was (#424).
- **One command says whether a change cost a search the previous binary
  answered.** `tools/memory-bench/precision.py` loads two corpora — the
  ratchet's 117 questions and the held-out comparison corpus's 56 — into a
  store per binary, asks both the same questions at the limit an agent gets and
  at the harness's twenty, and prints the paired rank of every question that
  moved, so a drop another query's gain cancels is still visible. It exits
  non-zero when hit@1 or MRR is lower on the second binary at either limit, and
  prints what one query is worth on each corpus (0.86 points on 117, 1.79 on
  56) because a smaller difference is not evidence from that corpus. CI builds
  the pull request's merge base and runs it beside the ratchet, and runs the
  harness's own Python tests, which ran nowhere before. The harness no longer
  restates the default limit: `quality.py` reads it from the product's
  `DEFAULT_SEARCH_LIMIT` and is the one place the twenty lives, with a test
  that holds the number to what a search with no `limit` returns (#452).

### Fixed

- **Uninstall leaves a package-managed binary to its package manager.** `leteo
  uninstall --yes` deleted whatever `current_exe()` pointed at, a Homebrew or
  mise/asdf and an npm install included, and the manager's record then
  disagreed with the disk — the next upgrade or reinstall acted on a file that
  was gone. The removal now recognises the same layouts `setup` already knows
  and reports the command that removes it (`brew uninstall leteo`, `mise
  uninstall leteo`, `npm uninstall -g …`, or `npm cache clean --force` for the
  `_npx` cache) instead of taking the file (#435).
- **Revising a memory counts as saving, and the reminder knows it.** The "you
  have not kept anything" line read the newest memory's `created_at`, and a
  topic-key revision moves only `updated_at` — so an agent that spent half an
  hour refining one memory was told, on every prompt past the fifth minute, that
  it had kept nothing. One aggregate over the later of the two columns answers
  the question, which also stops the reminder selecting every column of a row to
  read one timestamp (#439).
- **A hard delete confirms the tombstones it also erases.** The terminal's
  confirmation counted live memories and the delete removed the soft-deleted
  rows as well, so a project with three live and two tombstoned memories said
  "3 memory(s)" and destroyed five — and the two it left out were exactly the
  ones still recoverable. The confirmation and the delete now read one count, and
  the tombstoned rows are named on their own line (#438).
- **A hook writes its memory and leaves the vector to the backfill, and a search
  that has answered exits.** `subagent-stop` loaded the 13 MB semantic model and
  ran it inside the budget the agent is counting down — the one process the code
  said never reached that stage — and every `leteo search` embedded up to 1,024
  memories *after* printing its answer, so a one-shot command lingered for work
  nobody had asked it for. A hook now writes without a vector and leaves it to
  the backfill; the MCP server's background backfill fills them, and
  `doctor --repair` is the command for somebody who runs no server. Measured on
  a copy of a real store with the model present, a `subagent-stop` with five
  learnings took 108 ms and takes 70 ms (#422).
- **Session start resolves one directory instead of every stored one.** Deciding
  whether a directory's project drifted compared the recorded directory with the
  current one row by row, and both sides went through `realpath` on every row —
  filesystem work proportional to the store's history, on the path that runs
  before an agent sees a prompt. The directory is resolved once per call and each
  distinct recorded spelling at most once: `leteo hook session-start` on a copy
  of a store with 2,668 sessions made 5,228 `realpath` calls and makes 458, with
  the same answer (#421).
- **The pre-migration backup is complete before it carries the name a later
  open trusts.** `VACUUM INTO` wrote straight to `leteo.db.pre-schema-N`, so a
  process killed mid-copy left a partial file there, and every later open
  skipped the copy because the name existed: on a copy of a real store, a
  killed open left 2.5 MB of a 208 MB store under the final name, and SQLite's
  answer to it was "file is not a database". The copy is written under a
  working name of its own and renamed into place, and an existing file is taken
  for a backup only when it reads as a database at the schema it is named for.
  Two processes opening just after an upgrade no longer collide either: 7 of 20
  paired opens of a store at the previous schema failed with "open Leteo store"
  before this change and 0 of 20 after, because `VACUUM INTO` refusing the
  other opener's destination is not `SQLITE_BUSY` and the open's retry loop
  never saw it (#423).
- **A comment in a configuration file survives `setup` and `uninstall`.** Setup
  parsed `opencode.jsonc` with the comments stripped and wrote plain JSON back,
  so a person's notes about their own servers were deleted without a word, and
  `uninstall` could not parse a commented file at all: one comment in one
  config made a whole `leteo uninstall` exit non-zero for somebody who never
  configured OpenCode. One reader and one editor now serve every JSON file
  Leteo touches — `opencode.jsonc`, VS Code's `mcp.json`, ZCode's shared
  `config.json`, Pi's settings and the hook settings — and the writer splices
  the member or array element that changed instead of rendering the parsed
  document back. A file with nothing of Leteo's in it is left exactly as it
  stands rather than re-indented (#430).
- **Deleting a session or a project now takes a prompt's words with it, and
  refuses to let a peer bring them back.** Only a single prompt delete wrote a
  tombstone, and none of the five paths touched `sync_mutations`, where the full
  text of every earlier write sits until the journal is pruned: a prompt deleted
  with its session or its project returned on a peer's backfill or on re-import,
  and its words — a pasted secret is where this bites — stayed in the journal
  and were pushed to the cloud if nobody had acknowledged them yet. All five now
  call one function that writes the tombstone and rewrites each journalled
  payload to its identifiers, its date and the delete flags, the way a
  hard-deleted memory already did; the pulled session delete does the same for
  the prompts it takes. The redaction moved into `store/journal.rs`, so the two
  entities share one implementation of it (#455).
- **The README's benchmark numbers say what the document says, and the
  comparison page's generated block cannot be edited by hand.** Three sentences
  written in the pull request that refreshed the benchmarks described a page
  that had moved: the 20,204-token session was called "the fixed cost" (5,923 of
  it is schemas and instructions, 7,910 with the session start), the August
  finding that the agent without memory answers confidently was stated as
  November's, when it said it could not find it, and "Engram and two other
  engines" faced a table holding four. The MCP badge said 22 tools where the
  router serves 23, and a test now reads both halves of it against
  `list_all()`. `compare.py` records the digest of each block it writes in
  `docs/comparison.generated.json`, and a guard refuses a page whose block is
  not the one it wrote (#453).
- **Releases no longer run whatever `mcp-publisher` `latest` serves that day.**
  The registry job holds `id-token: write`, so the binary it piped out of a
  `releases/latest/download` URL ran with the identity that publishes
  `io.github.asanabrial/leteo`. The step now names v1.8.16 and checks the
  archive against a sha256 recorded in the workflow for each platform the step
  supports, and stops on a mismatch or an unknown `uname` pair rather than
  extracting anything. Bumping the publisher is now a reviewed change (#437).
- **A memory can no longer supersede itself, directly or through a cycle.**
  `mem_compare` took the same id twice and the pair stored, and nothing refused
  a `supersedes` that walked back to its own source. `not_superseded` hides
  every target of a judged `supersedes`, so either shape made the memories
  vanish from search, from the context block and from the prompt hint, with no
  listing that could show them again. The self-pair is now refused in the
  store, which every door shares — `mem_compare`, `mem_judge`, `save_relation`,
  import and replication — and a verdict that would close a cycle is refused
  with the memories already on it named in order. A backup or a peer that holds
  a self-pair has it dropped and counted rather than restored, and `leteo
  doctor` reports a supersession cycle or a self-pair an older build left in the
  store, since answering `not_conflict` on one of its pairs is the way out.
- **Uninstalling no longer takes the migration backups with it.** Both fallback
  scripts removed `leteo.db*` and `store.db*`, which is every
  `leteo.db.bak-before-migrate` and `leteo.db.pre-schema-N` beside the store —
  the copies `leteo uninstall` itself deliberately keeps, and the only way back
  from a one-way migration. Each script now names the files the binary owns, and
  `tests/uninstall_files.rs` fails when the list and a script disagree or a
  script reaches for the glob again.
- **Enrolling a project no longer queues relations the server refuses.** The
  enrolment backfill selected every relation but an `orphaned` one, which
  includes the `pending` candidates `find_candidates` files for an agent to rule
  on; the server requires the provenance a verdict writes, so it refused the
  payload on every attempt, and because push precedes pull the peer stopped
  replicating in both directions. The backfill queues only the statuses the
  journal carries, read from the one list both sides apply.
- **A save never lands on a memory a later one has overtaken.** A `mem_save`
  under the topic key of a memory `mem_judge` had marked superseded rewrote
  that memory, and an identical save inside the dedupe window folded into it:
  both answered as if the save landed, and every listing hides such a memory,
  so the text was unreachable from the moment it was stored. Both lookups now
  read the same narrowing the listings apply, a live copy of the key wins over
  a hidden one, and the save inserts a new memory when only a hidden one
  matched. Passive capture's duplicate check follows the same rule, and the
  reply still says the text was overturned, so a learning is stored rather than
  counted as a duplicate of a memory nothing shows.
- **A memory saved under another project no longer makes the directory it was
  saved from refuse every later write.** `mem_save` and `leteo save` naming a
  project the directory does not resolve to recorded that directory on the
  other project's manual session, and the check for a renamed repository read
  the row as proof of a rename: on one real store a single such save had every
  write that named no project answered `ambiguous_project` seven months later,
  with 331 sessions in the directory under the name it resolves to. Saved from
  inside a repository, the manual session of another project now records no
  directory. For a store that already
  holds such a row, answering the refusal with the detected project, the reason
  and the recovery token settles it: the other project's sessions keep their
  project and their memories and stop being counted as that directory's
  history, so the question is not asked again.

## [0.5.0] - 2026-11-22

Semantic search indexes multiple windows of each memory. The release also
improves session capture, replication, and cloud request handling.

### Upgrading from 0.4.3

The first open upgrades the SQLite store from schema 22 to 27. Take a backup
before upgrading if you may need to go back. Leteo 0.4.3 can open that backup
and will refuse the migrated store. With the semantic model installed, vectors
rebuild in the background or with `leteo doctor --repair`.

### Added

- **`leteo doctor --citations` lists the memories that cite a commit or a path
  which no longer resolves.** A memory that names a commit or a file was shown
  as current however long ago that thing went away: three of twelve sampled
  memories cited a hash that resolves to nothing, and 986 of the 7,274 live
  memories on one real store hold a forty-character hash. Run inside a
  repository, the check reads that project's own memories only, asks git about
  every hash in one process, looks for each backticked path in the working tree,
  and reports the counts with the first ten memory ids. It is a warning, never
  an unhealthy store, and nothing is hidden, marked or written. A file name
  written without its directory resolves when any tracked file carries it, and
  a path git ignores is skipped, each decided in one `git` process for the
  whole check, so it makes three at most. It is opt-in
  because it runs `git`; where it cannot (no git, no repository, a directory of
  another project) it says so and does not pass (#341).

### Changed

- **Search by meaning reads the whole of a memory, not its first 128 tokens.**
  A memory is now kept as up to 32 vectors, one for each 128-token window of
  its body with the title at the head of each. A question about something
  written further down, or asked in another language, can now find the memory:
  on one real store the median body was 548 tokens and 98% were longer than one
  window. A memory the words already found is still ranked by its opening, so a
  boilerplate paragraph at the end of a body cannot lift it. Migration 24
  rebuilds the vector table, and the vectors are made again in the background
  or by `leteo doctor --repair`. The cost is store size and search time:
  measured on 5,000 memories of 420 words, about six times the vector bytes
  (6,155 a memory instead of 1,024, about 30 MB instead of 5 MB) and about
  twice the time the stage adds to a search (20 ms instead of 7 to 12) (#325).

### Fixed

- **A memory found by meaning no longer takes first place from one found by
  its words.** When a search merges the two, the best of each list scored the
  same and the older memory went first, so the top result was often a guess.
  At equal score the memory the words found now leads (#320).
- **A message the agent harness wrote is no longer kept as something you
  asked.** Claude Code delivers its own notices through the prompt event, and
  on one real store 7,104 of 15,158 stored prompts were `<task-notification>`
  alone, about 39 MB of 42 MB, each one also searched for a hint. They are now
  neither stored nor searched, and the recent prompts a session opens with
  leave out the ones already stored, without a migration (#321).
- **Deleting a merged memory no longer loses the ones it replaced.** After
  `consolidate`, soft-deleting the result left both originals in the store and
  out of every search and listing, because the verdicts saying they were
  replaced outlived the memory that replaced them. A memory is now hidden only
  while the one that superseded it is live (#322).
- **The opening block shows the memories it promises.** When a block ran over
  its 4,000 bytes it cut memory titles first, so on the four largest projects
  of one real store 15, 10, 14 and 10 of the thirty were left out while old
  prompts and sessions kept their room. Prompts now give way first, then all
  but the newest session, and memory titles only after that. A session with no
  summary is no longer listed as a bare date, and `slim` has a 3,000-byte
  ceiling instead of 15,000, which was almost four times `full`'s (#323).
- **A memory is stemmed in the language it is written in, without setting
  anything.** With the `language` setting unset, which is the default, every
  memory was recorded and stemmed as English: on one real store all 7,274 rows
  were `en` while about 1,500 of the memories were in Spanish, so a question with
  another ending than the memory's (`migraciones` for `migración`) found
  nothing. Each memory is now read when it is saved, by counting the commonest
  function words of the twelve languages Leteo can stem, and recorded under the
  language that clearly leads; short or mixed text stays English. A named
  `language` still wins. Migration 23 changes the triggers only, so memories
  already saved keep what they have until `leteo doctor --repair` re-stems them,
  and `leteo doctor` says how many are waiting (#324).
- **Opening a session no longer moves another checkout's memories into this
  project.** When a directory's name was also the name of a project recorded
  from somewhere else, the hook merged the whole project, taking the other
  checkout's memories and prompts and the `_` and space spellings of the name
  with them, without asking or saying so. It now moves only the sessions
  recorded from this directory, with their memories and prompts, leaves the
  spellings alone, and warns with the number of sessions it left behind (#328).
- **Moving a memory out of a synced project no longer deletes it here.** The
  move queues a delete under the old project so the peer drops its copy, the
  pull hands this machine its own push back, and the delete was applied by
  identifier alone, so the memory vanished from the project it had just been
  moved into. A delete now applies only to a memory still in the project it was
  filed under. One cost: a delete from a peer that has not seen this machine
  merge or rename a project is filed under the old name and is ignored (#329).
- **`--data-dir` and `--database` now beat the environment.** With
  `LETEO_DATABASE` naming one store, `leteo --data-dir B stats` reported that
  store's contents and created nothing in B, because the variable was read as
  though it had been typed. A flag now wins over either variable, and the
  variables apply only when no flag names a store. The test suite no longer
  inherits either variable from the developer's shell, so a test that passes a
  flag cannot end up in a real store (#326).
- **The prompt that asks a model to compare two memories no longer defines
  `supersedes` backwards.** It said B was the later note and A out of date,
  while the scan stores A as the source and the store hides the target, so a
  model that followed the written definition would have hidden the current
  memory and kept the stale one. The five verdicts on the measured store all
  came out the right way round, because the model read the verb in its ordinary
  sense, so nothing was lost; the definition now says A is the later note. The
  prompt is also filled in one pass, so a memory that quotes the template, such
  as `{b_content}`, no longer has the other note's body pasted into its own
  section (#327).
- **A memory filed into another project's session no longer stalls replication
  or breaks a prune.** A session was queued only under its own project, so
  when the memory's project was the one enrolled the memory reached the peer
  without it and failed the foreign key on every pull, with no way out short of
  editing the store. The session now travels under the project of each memory
  or prompt that points at it, once. Deleting a session filed each memory's
  delete under the session's project, where it was dropped when only the
  memory's was enrolled; it now uses the memory's own. And `leteo projects prune` on a
  project whose session another project's rows still use aborted with a bare
  `FOREIGN KEY constraint failed`; it now leaves that session, as
  `delete_project --hard` already did (#330).
- **Merging a project no longer discards what had not been sent.** The merge
  queued the live memories again and then deleted everything still waiting under
  the old name, so a memory deleted before the next sync stayed alive on the
  other machine, and the judged relations and the previous versions of a memory
  waiting there were lost with it. What was waiting now moves to the new name,
  and the delete says it is filed under it; a peer that has not merged yet still
  holds the memory under the old name and ignores that delete until it does. The
  same went for moving a directory's sessions out of a project that was left
  empty, and for enrolling a project a second time, which dropped its unsent
  deletes. Enrolling also queues each memory's previous versions, which the
  catch-up never did (#331).
- **A hard delete no longer leaves the earlier text of a memory behind.** The
  version history is keyed by the memory's id with no foreign key, so deleting
  a memory, a session, a project, or applying a delete from another machine
  removed the row and kept every title and body it had been before; someone who
  hard-deleted a memory because an old revision held a secret had not removed
  it. The four paths now delete the versions in the same transaction, and a
  soft delete still keeps them (#332).
- **A `supersedes` verdict no longer hides the memory that overtook the other.**
  A pending pair is filed from whichever memory the scan met first, and
  judging it found the row in either direction and rewrote only the verb, so
  "the new memory supersedes the old one" was stored as the opposite and the
  current memory disappeared from search and listings. A `supersedes` verdict
  now turns the row round to the pair as given; the verbs that have no
  direction keep the order they were filed in (#333).
- **The review queue and the timeline now say what they published.**
  `mem_review` with no project listed and counted the whole store while the
  opening block counted this project's and sent the agent to it; it now answers
  for the detected project like the other reads. Marking a memory reviewed took
  no expected project, unlike pin, delete and update, so it now requires
  `expected_project` and refuses another project's memory with
  `project_mismatch`; it also returned the whole body where the description
  promises a 400-byte preview (5,499 bytes for one fixture) and carried no
  caveats where the list does, and now does both. A `mem_timeline` neighbour
  carried no caveats, so an overturned one read as current; each now carries its
  own. The "elsewhere" count of an empty search was measured against the
  caller's limit while the store stops at twenty, so `limit: 50` over more than
  twenty matches in other projects said "20" where it means "20 or more", in
  the tool and in `leteo search`. And the timeline's `project` is described as
  what it does, which is refuse a mismatch, not as ignored (#334).
- **The OpenCode and Pi plugins no longer drift from the hooks.** OpenCode
  declared the `post-compaction` event and never ran it, so after a compaction
  nothing was rebuilt, the list of memories the hint had named was not cleared
  and the language directive was not repeated; it now runs the hook when
  OpenCode reports `session.compacted` (on 2.x too) and hands the answer to the
  agent on its next turn. Pi sent every tool result over fifty characters to
  `subagent-stop`, so reading a file with a `## Learnings` heading stored its
  text as memories and every tool call started a process that writes to the
  store; it now sends only a subagent's result. And Pi, which stores the
  compaction summary itself, was then told by the recovery text to store it
  again: the hook input takes `summary_stored`, and the text leaves the
  instruction out when the host says it already did (#335).
- **Setup no longer detaches a symlinked config or loosens its permissions.**
  Every file Leteo edits in place was written by renaming a temporary file over
  it, and a rename onto a symbolic link replaces the link: a `CLAUDE.md` kept in
  a dotfiles repository became a plain file and the repository never saw the
  change, and the new file took the umask default, so one restricted to `0600`
  became readable by the group and by everyone else. The write now goes through
  the link into the file it names, with the temporary file beside that file, and
  the file keeps its mode. A link to nothing, a loop of links and a link to a
  directory are refused with a message instead of being replaced (#336).
- **`doctor` no longer counts a check that could not run as one that passed.**
  Four checks turned a failed query into the answer a healthy store gives: no
  topic key shared, no memory type unfiled, no stale hash, no full-text trigger
  missing; and the stems row count compared two failures as equal. Each now
  reports that it could not be checked, with the error, as a failure, so the
  store is not called healthy over something nobody looked at. A memory with no
  hash at all, which a database adopted from an early Engram schema carries, was
  also dropped from the stale-hash count and skipped by `--repair`; it now counts
  as stale and is repaired (#337).
- **The prompt hint no longer repeats itself.** It named memories the opening
  block had just listed, because neither a session start nor a compaction
  recorded what the block carried as already shown; both now do, from the
  memories the byte budget actually left in the block rather than the ones it was
  asked for. A memory with a judged conflict was named again, with its caveat,
  on every matching prompt, because any caveat lifted the already-shown rule; a
  caveat now lifts it once, and a new one lifts it again. The 110-byte header
  sentence was repeated on every hint; it is now said once per conversation and
  again after a compaction, with a short line in between that keeps the hedge.
  The hint state is also written through a temporary file and a rename, so a
  concurrent hook no longer reads an empty file, and a state file from an older
  Leteo still reads. A prompt that is only an acknowledgement such as "continue"
  is still searched as before: a gate for those is left for a measurement on a
  hand-labelled prompt set rather than a guessed threshold (#339).
- **A save that names no session joins the one the agent is in.** The server
  never learns the agent's own session id, so every `mem_save`,
  `mem_session_summary`, `mem_save_prompt` and `mem_capture_passive` without a
  `session_id` landed in `manual-save-<project>`: on a copy of a real store 451
  of one project's 471 memories hung from it, and "Recent Sessions" showed one
  line for four months of work. They now go to the project's most recently
  active open session if it did anything in the last 30 minutes, the window a
  prompt is attributed within, and to `manual-save-<project>` otherwise. The
  reply names the session when the caller did not. Two agents working one
  project at once share the most recent session, which the tool description
  says. Memories already filed are not moved (#340).
- **A settings file with a typo is no longer rewritten as defaults.**
  `leteo setup --language` and `--context`, and the setup wizard's save, read
  the file, changed one answer and wrote all of them back. A file that was not
  valid JSON (one trailing comma) read as every default, so the write replaced
  the voice, the interface and the rest; a field of the wrong type was saved as
  its default. They now stop with the file's path and the reason and leave it
  as it was. A key this version does not know is kept when the file is
  rewritten, so a newer Leteo's setting survives an older one. `"voice":
  "Quiet"` and `"context_size": "DEEP"` were read as the defaults by every hook
  while the command line accepted them in any case; they are read in any case
  now. The system language is read from `LC_ALL`, `LC_MESSAGES`, `LANG` and
  `LANGUAGE` in that order, the one POSIX gives, and a variable that names no
  language Leteo knows, such as `LANG=C.UTF-8`, is skipped instead of ending
  the search (#338).
- **The local HTTP API no longer calls a busy store a bad request, and no
  longer holds a silent connection forever.** `leteo serve` answered every tool
  error `400`, including `store_busy` and `store_unavailable`, which are the
  server's condition and not the request's, so a client could not tell a call
  worth repeating from one it had got wrong. Those two now answer `503`; every
  other refusal stays `400`. Reading a request had no limit either: a socket
  that sent nothing, or stopped half-way through its body, stayed open for as
  long as the client cared to keep it. A request that has not arrived in full
  within ten seconds is now answered `408` and closed (#342).
- **A backlog of queued captures no longer delays the next prompt.** Every hook
  replayed the captures a busy store had left beside it before it answered, the
  prompt hook included, and the replay was bounded only by the hook's own
  budget, so after a busy period the message a person typed waited on old
  captures (about 2 ms each, measured: a full spool of 1,000 took 2.2 s). The
  replay now happens on `session-start`, `subagent-stop` and `session-stop`;
  `user-prompt-submit` and `post-compaction` leave the queue alone (#343).
- **A hard delete no longer leaves the memory's earlier text in the sync
  journal, in swept-up leftovers, or open to a late replicated version.** After
  #332 removed a memory's version rows, `sync_mutations.payload` still held
  every earlier title and body, sent or not; version rows orphaned by hard
  deletes made before then stayed in existing databases; and a version that
  reached a peer after the delete re-created its row. A hard delete now cuts the
  memory's journal payloads down to the identifiers a peer needs and records a
  tombstone (migration 25) that makes a replicated version for that memory a
  no-op. `leteo doctor` reports orphan version rows as `orphan_versions`, and
  `--repair` removes them (#368).
- **A project merge now reaches a peer as a rename, so deletes and updates filed
  under the new name find their rows.** A peer that had not merged held every
  row under the old name and ignored a delete filed under the new one, for good.
  A merge queues one `project_rename` mutation ahead of the rows it queues
  again, and a peer applies it by moving the rows without queueing anything. A
  delete that was waiting is queued again after it; unsent mutations under the
  other spellings of the name are carried; and the versions of a moved memory
  travel with it. A session's delete and its later updates, such as
  `end_session`, now go to every project the session was sent to, and
  `projects prune --apply` skips a project holding soft-deleted memories and
  says so instead of aborting. An older peer or cloud server does not know the
  new mutation and refuses it until it is updated (#369).
- **On OpenCode and Pi a memory the hint named once can be named again.** Both
  hosts put the hint in the system prompt of one turn, but the hook recorded
  every memory it named as shown for the whole session, so a memory the agent
  had already lost was withheld from it for good. The plugins now say the hint
  is transient, and the hook then records neither the memories it names nor
  the caveats it delivers, and says the full header each time. Pi says the same
  of its opening block and of the block after a compaction, which it also
  delivers for one turn; OpenCode keeps its opening block and says nothing
  there. Pi's protocol no longer tells the agent to persist the compaction
  summary Pi has already stored, and the OpenCode 2.x adapter also runs the
  compaction hook for `session.next.compaction.ended`, once per compaction
  when the host emits both events (#370).
- **A rename a peer sends moves only the projects this store replicates.** The
  apply side trusted every name the rename listed, so a pulled rename of a
  shared project could name a project that exists on this machine alone and file
  its memories, sessions and prompts under the shared one, from where every
  later edit was queued to peers. A name that is not enrolled here is now
  skipped, without refusing the rest of the pull, and a rename that skips every
  name creates and enrols nothing (#381).
- **Two tests that failed now and then no longer do.** The PostgreSQL tests
  named their principals after the microsecond they started in, and tests run
  as threads of one process, so two could draw the same name; revoking one
  session then signed the other out and its request came back 303 where it
  expected 400. Each test now draws a stamp no other can. The attribution test
  saved a memory with no session, which joins the project's most recently
  active session, with activity read to the second, so a clock tick between two
  writes decided whether it landed in a session whose prompt had no window; it
  now ends both sessions first, so the answer no longer depends on the tick
  (#377).
- **A save that lands on a memory already stored says when a later one has
  overturned it, and `leteo save` joins the live session.** `mem_save` and
  `mem_session_summary` named the existing memory of a duplicate or a revision
  with no caveats, so a reply could present a superseded decision as standing;
  they now carry `caveats` in that case and say nothing on a fresh save. `leteo
  save` with no `--session` filed under `manual-save-<project>` beside a session
  a hook had opened; it now joins the project's most recently active live
  session as `mem_save` does, and `leteo prompt` and `leteo consolidate` with
  it. `--session` still wins (#373).
- **A manual `supersedes` verdict says which memory supersedes, and one pair
  holds one relation.** `mem_judge` judged a row by its own id and kept the
  direction the scan had filed it in, so a `supersedes` could hide the current
  memory behind the one it replaced; it now takes `superseding_id`, required for
  that verb, and a call without it is refused naming the two ids that would be
  accepted. The server instructions, the tool descriptions, the opening
  handover and the three skills say so. A relation arriving from a peer for a
  pair already held under another `sync_id` used to be inserted as a second row,
  possibly in the opposite direction, and both counted; it now updates the row
  here when it is newer and is dropped otherwise (#374).
- **Three more `doctor` checks say they could not run instead of passing or
  misreporting.** `hook_spool` read a spool directory it could not list as
  nothing waiting, and `settings_readable` read a settings file it could not
  read as nothing ignored, while `load` was using the defaults for it; the
  three full-text sync checks printed a failed count as `fts=-1` in a
  row-mismatch report. Each now says what could not be checked, with the
  error, and the store is unhealthy. A missing spool or settings file is still
  an empty one, and a hook that cannot list the spool says so on stderr and
  replays nothing (#375).
- **`leteo serve` drops a client that does not read, bounds its connections,
  and no longer parks a runtime worker behind a busy store.** A response was
  written with no deadline, so a reader that never drained held its task once
  the answer outgrew the socket buffer; every accepted connection got a task
  with no limit; and a tool call waited on the store's lock and SQLite's busy
  timeout on the worker that was serving other connections. A response now has
  the same ten seconds as a request, at most 128 connections are served at
  once and the next is answered `503` and closed, and tool calls run on the
  blocking pool (#376).
- **A memory records the directory it was written from.** A save with no
  session joins whichever session is live in its project, and a session keeps
  the directory of whoever opened it, so on a real store 585 memories hung from
  a session that recorded `H:\REPO` and another still named a worktree deleted
  months earlier, while the reply to each save told the agent the directory of
  that call. Migration 26 adds a nullable `observations.directory`; `mem_save`,
  `mem_session_summary`, `mem_consolidate`, `mem_capture_passive`, `leteo save`,
  `leteo consolidate` and the subagent-stop hook (live or replayed from the
  spool) write it, and it travels with the memory through replication, the
  cloud codec, export and import. A memory with none — everything written
  before this, whose origin was never recorded — says so rather than inheriting
  its session's. `mem_get_observation` names it; the MCP listings and search
  results do not, a path on every hit being the reply size they are bounded
  against. `sessions.directory` is untouched by a later save (#315).
- **An empty session a hook opened no longer makes its directory a project.**
  A hook opens a session wherever an agent starts, and each one made its
  directory's name a project that `mem_save` accepted when an agent wrote it:
  on a real store 629 of 651 sessions opened under temporary directories held
  nothing, across 14 names such as `tmp` and `bin`. A name is now known when it
  holds a memory, a prompt, or a session opened by `mem_session_start` with a
  project of its own, so creating a project there and saving to it later still
  works. Migration 27 adds `sessions.declared` and marks every existing session
  declared, so no name known before the upgrade becomes unknown; the column is
  not replicated. A name this directory was itself recorded under still resolves,
  because the drift refusal offers it and a passive capture files under it
  (#316).
- **`leteo session-start --project` makes its project known, as
  `mem_session_start` does.** It opened an undeclared session, so a project
  created from the command line was refused as unknown when an agent named it
  from another directory before anything was saved to it (#391).
- **Behaviours the reviews of #321 to #343 recorded as untested now have a test
  that fails when they are broken.** Nothing in the program changes except a
  doc comment on the stop-word table that named example words the table does not
  hold, the one expression `restem_observations` wrote four times, and the
  comments that said the next hook replays a captured learning, which the prompt
  and compaction hooks do not. The tests cover a language setting no stemmer
  exists for, a changed windowing, an empty `LETEO_DATABASE`, a delete naming no
  project, a merge into a project nobody replicates, a prompt's session in
  another project, the caveats of a timeline's later neighbours and of an
  opening block, `mem_review` and the command line's elsewhere count where the
  directory names no project or the limit is past the store's, hash repair
  failures, a live session joined by a passive capture and a prompt, uninstall
  through a linked instruction file, and a session start replaying the spool.
  The git repositories built by tests no longer depend on a signing
  configuration, and a test that asserted nothing now asserts that git failing
  is an error. Two defects they found are left for their own issues: a harness
  prefix after a non-ASCII space, and a caveat lookup that fails silently in
  `mem_timeline` (#372).
- **A stored message the harness wrote is hidden from the recent prompts
  whatever space it starts with, and a tool that could not read a memory's
  caveats says so.** The listing skipped a fixed set of ASCII whitespace in SQL
  before a harness prefix while the hook used Rust's `trim_start`, so a stored
  `\u{a0}<task-notification>` was a harness message to the hook and a prompt to
  the listing; both now read `is_harness_message`, and the listing streams rows
  until the page is full rather than over-fetching a fixed number. Separately,
  `mem_search`, `mem_context`, `mem_review`, `mem_timeline`, `mem_get_observation`,
  `mem_update`, `mem_save` and `mem_session_summary` answered an ordinary reply
  with no caveats when the lookup failed, which reads as "nothing was said
  against these" and hands an overturned memory over as current. Each now keeps
  answering and adds a sentence to its `hint`; `mem_review` and `mem_timeline`
  gain an optional `hint` field, skipped when empty (#394).
- **The opening block, the per-prompt hint and the terminal interface say when
  they could not read a memory's caveats.** They answered a failed lookup with
  no caveats at all, so an overturned decision read as current in the two texts
  an agent reads most. The opening block now carries the same sentence the MCP
  tools use, once, under its heading and inside its byte budget; the hint
  carries it once under its header; opening a memory in the terminal interface
  puts it in the status line, in the interface language. The rule moved from the
  MCP layer to `recall`, which all of them can reach (#396).
- **The cloud server bounds a request and says "busy" when the database is.**
  It had no deadline per request and no limit on how many ran at once, and a
  database that could not take a connection — the pool's five-second acquire
  timeout, a full server, an unreachable one — was answered `500`, the same as
  a fault. A request now has 25 seconds from arrival, queueing included, and 64
  run at once; past the deadline, and for those database errors, the answer is
  `503` with `Retry-After: 5`. The sync client already retries any answer other
  than 401, 403 and 409 on its exponential backoff, so it needed no change.
  Beside it, a store error while checking the bearer token, the project grant
  or the dashboard login was reported as 401 or 403 and would have sent a
  client into its long standing backoff over a busy database; it now answers
  as the store error does. Both limits are in the README (#388).

## [0.4.3] - 2026-11-03

A worktree is known by its repository, and a project name that is only a guess
says so.

### Added

- **Every release is submitted to winget.** The manifests live in the repository
  and are filled in for each tag from its own `SHA256SUMS`; winget's maintainers
  review each one as a pull request, so a version can reach it later than the
  other registries (#308, #310, #312).

### Fixed

- **A worktree no longer becomes a project named after its folder.** Its identity
  depended on running git, and any failure silently produced the folder's name,
  reported as a successful detection. It now comes from the `.git` link and the
  repository's shared config, read from disk. A worktree with no `origin` takes
  its main checkout's name, a link whose target was moved or pruned still names
  the repository it points into, and a worktree inherits an uncommitted
  `.leteo/config.json` from its main checkout. Sessions already filed under the
  folder's name are folded into the right project when the worktree is reopened
  (#314).
- **A parent directory of worktrees offers projects rather than branches.**
  Children are named and folded by that same identity, so a folder holding
  several worktrees of one repository is that repository instead of an ambiguous
  choice between its branches (#314).
- **A project name that could not be established says it is a guess.** When
  neither the repository nor the directory can be read, detection answers
  `dir_basename` with a warning instead of passing for a finding, and a hook uses
  the project that directory's sessions recorded before accepting the guess
  (#314).

## [0.4.2] - 2026-10-17

Claude Code with a relocated profile gets all of Leteo in that profile.

### Fixed

- **`leteo setup claude-code` follows `CLAUDE_CONFIG_DIR` for the server and the
  instructions, not only the hooks.** With the variable set, the hooks went to
  `$CLAUDE_CONFIG_DIR/settings.json` while the server entry went to
  `~/.claude.json` and the protocol to `~/.claude/CLAUDE.md`, files the Claude
  Code using that profile does not read: it had hooks calling a server it was
  never given. Both now go inside `$CLAUDE_CONFIG_DIR`, beside the hooks; unset,
  nothing moves. An entry an earlier setup left in `~/.claude.json` stays there;
  `leteo setup claude-code` again writes the one this profile reads (#306).

## [0.4.1] - 2026-10-17

The Pi package is on npm, and this release keeps it from being installed twice.

### Added

- **The Pi package is on npm as `@asanabrial/leteo-pi`, and every tag publishes
  it.** `pi install npm:@asanabrial/leteo-pi` is a second route beside `leteo
  setup pi`, which stays the default. The registries workflow publishes it
  through npm's trusted publishing at the tag's version, as it does the wrapper
  (#273, #301).

### Fixed

- **`leteo setup pi` no longer adds a second Leteo beside the one from npm.**
  With both listed, Pi 1.1.0 loads the first and refuses the second at every
  start with `MCP server "leteo" is already registered`. Setup now refuses when
  Pi's settings list `npm:@asanabrial/leteo-pi`, writes nothing, and names the
  entry and `pi remove` as the way out; `doctor` reports a Pi that lists both,
  which is how a `pi install` after a setup shows up (#304).

## [0.4.0] - 2026-10-17

A package for Pi, a session that opens on thirty titles instead of fifty
previews, a quieter per-prompt hint, stemming for Catalan, Basque and Polish,
and replies that cost fewer tokens. **`mem_search` and `mem_save` reply with
less** — the fields removed are listed under Changed — so a client that read
`sync_id`, `rank` or the echoed memory from them reads them no longer. The
schema is still version 22: 0.3.0 and 0.4.0 open the same store, and going back
needs no copy. Inside Pi, upgrade the binary with the package: the guard that
keeps Claude Code's hooks quiet there is in this binary, and without it both
sets of hooks fire and every prompt is stored twice.

### Added

- **Pi loads Leteo's memory tools when a session looks for them, not on every
  request.** The package registered them `direct`, so the declarations of all
  twenty agent-profile tools were sent with every request of every Pi session. On
  Pi 1.2.0 and later it registers them `deferred`, and Pi's `tool_search` loads
  them on demand; a Pi below that keeps `direct`, which is what it has always
  had. Measured on Pi 1.2.0 with `opencode-go/deepseek-v4.1-flash`, an empty
  project, a throwaway store and Estigia's server configured beside this one,
  first-request prompt tokens were 13,832 with no Leteo, 20,037 with `direct`,
  14,797 with `deferred` and 15,963 with the four session tools kept `direct`.
  One recorded session under `deferred` called `tool_search`, then
  `mem_session_start`, then `mem_save` (#297).

- **The session cost the docs quote can be re-measured by anyone.**
  `tools/memory-bench/session_cost.py` runs both engines' hooks and MCP servers over
  throwaway copies of stores you name and totals what the model is sent in one
  session, with the schemas paid upfront or deferred; `--labels` replays the
  per-prompt hint over a labelled sample and reports how often it is silent and
  how often it names the right memory. `docs/does-memory-save-tokens.md` is
  corrected with it: the agent profile has 20 tools, not 19, and most of the
  `tools/list` bytes it called a fixed cost are output schemas the model is not
  sent (#198).

- **Pi gets a package that registers Leteo's MCP server itself.** `leteo setup
  pi` writes `plugin/pi` into Pi's agent directory, lists it under `packages` in
  `settings.json` (idempotent, and the other settings and packages are left as
  they were), and removes the `leteo` entry an earlier setup wrote in
  `mcp.json`, since a server of that name there overrides the package's
  registration. The package opens and ends the session, persists prompts,
  captures Key Learnings from tool output of more than 50 characters (Leteo's
  own tools excluded), saves the compaction summary, recovers context after it,
  appends the memory protocol to the system prompt, and registers `leteo mcp`
  with the `direct` exposure, so the model can call the tools without searching
  for them. A reload does not end the session (#269).

- **The comparison page measures more than Engram.** The harness behind it now
  runs every engine through one adapter, which starts the engine isolated under a
  throwaway state, writes the held-out corpus through its own write path, searches
  through its own search path, and maps each hit to a corpus memory by provenance
  or by one engine-independent content rule. `docs/comparison.md` (formerly
  `docs/comparison-with-engram.md`) is regenerated by one command and now holds
  basic-memory and the reference MCP memory server beside Engram and Leteo, each
  row with its version and run date, the unmappable hits per engine, and any engine
  that could not run listed with the reason instead of as zeros. Engram's and
  Leteo's numbers are unchanged (#248).

- **The comparison can include engines that need a local model.** `compare.py
  --tier llm` runs mem0 (verbatim and with fact extraction), claude-mem and
  Graphiti MCP on a local Ollama, loads each from an empty store several times,
  and reports every figure as a mean with the lowest and highest pass, beside the
  models and digests used. An engine that cannot run is listed with the reason,
  not scored as zero. It is opt-in and not part of CI (#248).

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

- **Changing the memory language says what it leaves behind.** A memory keeps
  the language it was written in, and nothing re-stems the ones already saved, so
  the setup screen's language step now warns with the count — and only when the
  chosen language differs from the rows it would leave. It stays quiet when
  there is nothing the change would leave behind.
  `leteo doctor` reports the split as information, with the count per language,
  and still changes no memory's language under `--repair` (#185).

- **The per-prompt hint is quieter, and no longer tells the agent to read.**
  One relevance bar of 1.6 times the median now decides every candidate; the
  looser 1.2 bar for memories outside the session's opening block is gone. On
  170 prompts labelled by hand, replayed through the hook against a store cut
  back to each prompt's time, the hint is silent on 56 of the 100 that needed
  nothing instead of 23, names a labelled memory on 30 of 70 instead of 31, and
  spends 8,004 tokens over all 170 prompts instead of 16,102. Its first line no
  longer orders a `mem_get_observation`; it says the lines match the prompt's
  words and can be ignored. A cosine gate from the local embedding model, and a
  grid and a fitted scorer over several signals, were measured and none kept 30
  named at 80 silent, so the bar stays a plain margin (#198).

- **Breaking: `mem_search` and `mem_save` reply with less.** A `mem_search`
  result no longer carries `sync_id`, `session_id` or `rank` (no tool takes a
  sync_id, and the order of `results` already is the rank), and names its
  `project` only when it differs from the reply's and its `scope` only when it
  is not `project`. A `mem_save` reply, and `mem_session_summary`'s, which shares
  its shape, no longer echoes the memory with a content preview: `observation`
  is now `id`, `type`, `title`, `topic_key` when set, and `state` when not
  active, and `candidates[]` lose `sync_id`; `judgment_id` and `id` are what
  follow-up calls take, so judging works as before. Nothing in this repository
  read the removed fields. `mem_get_observation`, `mem_timeline`, `mem_context`
  and the other tools that return a memory are unchanged. Replayed against a
  copy of a real store of 1,048 memories (tiktoken `cl100k_base`), the mean
  `mem_search` reply went from 973 to 726 tokens over 20 queries and the mean
  `mem_save` reply from 508 to 292 over 5 saves (#291).

- **A session opens with thirty memories as titles, not fifty with previews.**
  The default `full` size now names thirty memories (50.4% of the questions
  measured against a real store, against 55.9% for fifty), and the opening block
  carries no content under any of them: the newest five no longer open with their
  first three hundred characters, and pinned memories are titles too. `slim` and
  `deep` keep twenty and eighty, and `mem_context` follows the setting for its
  count but keeps its own previews and its own byte ceiling. The markdown
  block's `full` ceiling was measured again at 4,000 bytes (3,425 for the
  thirty-memory block of a real store). The first thirty are the same memories,
  in the same order, as before. Measured on a copy of that store, session start
  went from 2,521 to 1,586 tokens, post-compaction from 1,991 to 1,056, and
  `mem_context` from 3,029 to 2,422. The skills now say every entry of the block
  is a title alone.

- **The opening block spends fewer tokens on line metadata.** A session line no
  longer repeats the project the block is already scoped to (a block with no
  project of its own, and any line from a different project, still names it) and
  no longer ends in `[N observations]`; session and prompt timestamps, in the
  block and in `mem_context`, are printed to the minute instead of the second.
  Which memories, sessions and prompts are selected is unchanged. Measured on a
  copy of a real store, session start went from 2,596 to 2,521 tokens,
  post-compaction from 2,066 to 1,991, and `mem_context` from 3,039 to 3,029.

- **basic-memory's row is measured over several passes, not once.** Its
  full-text index is filled asynchronously after `write_note` returns, and
  0.23.2 exposes no completion signal to wait on, so two loads of the same
  corpus are not the same store and their total hits differ. `bench_engines.py`
  now runs an engine that declares more than one pass into a state of its own per
  pass; the page's head-to-head row reports the mean with the lowest and the
  highest pass, the engines table the pass count, the per-kind table the mean
  alone, and a run that faults before its passes are done is not measured at all
  (#262).

- **A search no longer writes vectors; a memory gets its vector where it is
  written.** `mem_search` used to embed every memory in scope with no current
  vector and keep them while it held the store's single lock: on a 54k-memory
  store with no vectors the first fallback search spent seconds embedding and
  writing before it could answer, and a store it could not write held every
  vector in memory for the question. A save, a revision, a consolidation and an
  import now give their row a vector as they commit, a bounded backfill fills
  what is left — in the background for `mcp` and `serve`, after a CLI search, and
  on demand for `doctor --repair` — and the search stage only reads. The same
  first fallback search on the 54k copy is 0.25 s against 2.39 s and writes
  nothing. `doctor` reports the coverage as `semantic_vectors`. (#205)

- **The benchmark harness is `tools/memory-bench/`, formerly `tools/engram-bench/`.**
  It compares Leteo with every engine on `docs/comparison.md`, not Engram alone,
  and the old name hid where the other engines' benchmarks were (#267).

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

- **An import gives every memory its vector.** `import --from-engram` replaced
  the database before any embedder could see it, so an adopted Engram store
  came over with no vectors at all — 0 of 1,045 on a real copy — and was found
  by its words only until a backfill caught up. A JSON import embedded one
  budget step of 1,024 rows and left the rest. Both now embed every row before
  the command returns, and report what they embedded as `vectors`; without the
  semantic model the import still succeeds and says how many rows it left
  without a vector and why (#285).

- **Titles in the per-prompt hint and the verdict handover are bounded like the
  opening block's.** The hint's memory lines, the title of the other memory in a
  hint caveat, and the titles in the verdict handover printed the whole title,
  while the opening block cuts at 140 characters; 93 of the 4,756 titles in a
  real store are longer. All three now use the block's own rule.

- **Claude Code's hooks no longer register a second session inside Pi.** Pi
  runs Claude models through pi-claude-bridge, which loads the user's Claude
  Code settings, so every `leteo hook` it fired put the conversation in the
  store beside the one the Pi package owns. A hook now answers `{}` without
  opening the store when `PI_CODING_AGENT` is set, unless the caller names
  itself with `LETEO_HOOK_CLIENT=pi`, which the Pi package does (#269).

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
