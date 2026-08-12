# Changelog

All notable changes to Leteo are documented in this file.

## [Unreleased]

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
