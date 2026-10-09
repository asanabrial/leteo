# MCP tools

## Purpose

The surface agents actually use: an MCP server over stdio exposing the store as
typed tools. Everything here is answered in one round-trip and read by a model,
so the cost of a reply is part of its correctness — a payload that pushes the
useful part out of a context window has failed even if every field is right.

## Behaviour

1. **Twenty-three tools, one store, and `--tools` names which of them.** The
   profiles are `agent`, `admin` and `all`, and a single tool may be named
   instead. Anything else is refused at start-up with the profiles and the
   twenty-three spelled out: an unknown name used to be kept as though it were a
   tool, so it matched nothing and every route was removed. `--tools=agnet`
   started a memory server with no memory tools on it, in silence, and
   `--tools=AGENT` did the same — and what an agent sees then is "Leteo's tools
   are missing", which the skill answers with "run `leteo setup` and restart".
   A typo sending somebody to reinstall an install that was fine.

   Twenty-three tools. Writing: `mem_save`, `mem_update`,
   `mem_delete`, `mem_save_prompt`, `mem_session_start`, `mem_session_end`,
   `mem_session_summary`, `mem_capture_passive`, `mem_pin`, `mem_unpin`,
   `mem_judge`, `mem_consolidate`, `mem_merge_projects`. Reading: `mem_search`,
   `mem_context`, `mem_get_observation`, `mem_timeline`, `mem_review`,
   `mem_compare`, `mem_stats`, `mem_doctor`, `mem_current_project`,
   `mem_suggest_topic_key`.

   And every tool refuses a field it does not take, including the two that take
   none. A tool declared without a parameter type publishes
   `{"type":"object","properties":{}}` — an object schema with no
   `additionalProperties: false`, which tells a client extra fields are welcome
   and then drops them. `mem_stats` accepted `project` and answered with the
   whole store: 4,015 memories where that project holds 1,712, with nothing in
   the reply to say the narrowing had gone. Asking is the natural mistake,
   because every other read here takes a project — and the answer exists, under
   `mem_doctor`, which is where the description now sends it.

2. **Every tool answers with `structuredContent` against a declared
   `outputSchema`.** The types live in `src/mcp/output.rs`; nothing is assembled
   ad hoc into a string.

   And the reply validates against it, which is a stronger claim than declaring
   one. A field the reply may leave out must not be declared required: three
   were — `partial` on every search result and `content_truncated` on the memory
   three tools hand back — so a client that validates strictly rejected those
   answers outright. Held by a rule with no exception: a `skip_serializing_if`
   carries a `default`, so a field that stops being an `Option` cannot put the
   defect back without saying so.

   Validated all the way down, not only at the top. The guard compared the
   top-level `required` list and nothing under it, so the memories inside a
   search and the entries inside a timeline were declared and never checked:
   sending an `id` as a string or dropping a nested `session_id` left it green.
   It now follows `$ref` into `$defs` and walks `properties` and `items`,
   enforcing `required` and `type` at every level. What it reaches is decided by
   its fixture — with one memory in the store a timeline comes back as two
   empty lists and validates nothing, so the fixture holds enough of them to
   have neighbours on both sides.

   The schema describes the refusal as well as the answer. A failure comes back
   as `structuredContent` — it is what carries `error.code`, the
   `available_projects` an ambiguous directory offers and the `recovery_token`
   an agent replays — and it carries none of the fields the success shape
   declares required, so a client that validates rejected every error this
   server returns: twelve error replies out of twelve failed their own tool's
   schema, each on the first required field of the answer they are not. That
   client is not hypothetical; two defects here were found by OpenCode
   validating, and both were about the answer. The union is expressed through
   `required` alone, so the root keeps its `type` and `properties` for anything
   that reads a schema rather than validating against it, and it costs 1.4 KB
   across `tools/list` — 51.0 KB to 52.4.

   A refusal the parameters never got past is text and carries no
   `structuredContent`: `mem_timeline` handed a `limit` answers "unknown field
   `limit`, expected one of `id`, `observation_id`, `before`, `after`". That is
   the protocol layer refusing before any tool runs, and it names the accepted
   fields, which is what a caller needs. Everything a tool itself answers —
   including every failure it reports, `unknown_project` and
   `observation_not_found` and the rest — carries the structured error with its
   code, which is what the recovery flows read.

3. **Bodies are bounded, and the bound is published.** Any tool that hands back
   a body it did not promise in full cuts it at 400 bytes on a word boundary and
   says that it did, with `content_truncated` and with a sentence in its own
   description. `mem_get_observation` is the one tool that promises the body in
   full, and its description says so.

   The unit is bytes, and it is the unit the descriptions publish: a previewing
   description says `400-byte`, and a test holds every one of them to
   `PREVIEW_BYTES`, so the constant and the prose cannot drift apart. A
   description that promised characters was the bug this fixes — a 400-byte cut
   of non-ASCII text is fewer than four hundred characters, so the agent was told
   it had been shown more than it had. The cut itself is unchanged; only the
   promise moved to the unit already applied.

   This covers what a caller has just sent as much as what it is being shown.
   `mem_update` echoed a memory whole — 4,556 bytes to change a title, byte for
   byte what `mem_get_observation` sends — `mem_save_prompt` echoed the prompt,
   which is where people paste, and `mem_judge` echoed both the reason and the
   evidence: 24,359 bytes of the caller's own words to record one verdict.
   `mem_timeline` previewed its neighbours and handed its focus over whole,
   which its own description already denied. Nine tools preview; two guards hold
   the rule — one names them and one gives every surface twenty thousand bytes
   and requires a small answer, which is what found the ninth.

   That second guard measures what an agent receives. A reply carrying
   `structuredContent` also carries the same JSON serialised into a text block,
   because the protocol asks for one so that a client predating structured
   output — or one that reads only `content` — still gets an answer. So a
   20-result `mem_search` on a real store is 16,428 bytes of structured content
   and 16,957 of the same thing as text, 33,385 in all, and the guard weighs
   both.

   **The text half goes to every client.** The protocol says SHOULD rather than
   MUST, and the duplicate is what a client that reads only `content` needs in
   order to render anything at all. Revision `2025-06-18` introduced
   `structuredContent`, but it is each client's implementation that decides
   whether the model ever sees the structured half, and no client has been
   measured that reads it and ignores the text. So the JSON is sent beside
   `structuredContent` at every revision, refusals included — a refusal's
   `error.code`, its `available_projects` and its `recovery_token` are what the
   recovery flows read from the text block, and a pointer sentence in their
   place leaves an ambiguous directory unusable. The duplicate is withheld from
   no client until a measurement shows one that reads `structuredContent` and
   not the text.

   The cost is the duplicate, measured on the wire rather than in a model.
   Against a real store a 20-result `mem_search` is 32,097 bytes at every
   revision, where it was 15,917 when the text half was suppressed from
   `2025-06-18` on: the 16,180-byte difference is the same JSON sent a second
   time as escaped text. What the duplicate cost an agent in its own context
   was never measured — the `#112` entry in `CHANGELOG.md` read "about half",
   which is a wire fraction and not a token count, and no client was inspected
   to see which block reaches the model. Every revision rmcp knows is
   exercised in `tests/mcp_protocol.rs`, plus one it does not, which is answered
   at the `2025-11-25` ceiling: each reads the whole answer as text with
   `structuredContent` beside it, and a refusal carries its code. The guard's
   bar has not moved; the largest surface now sits at 5,858 bytes of the 8,000
   allowed.

   And so are the lists, at both ends. Every budget a tool takes has a ceiling
   and publishes it in its own schema, so a caller can plan against it: a
   ceiling that lives only in the code is one nobody can read. `mem_timeline`
   was given one after a window of a million came back with a whole session,
   191 KB, and `mem_context` — the only route to context eleven of the fifteen
   clients have — kept all three of its budgets open at the top. Asked for
   9,999 memories, sessions and prompts against a real store it answered with
   1,201, 212 and 120 of them, in one reply of 469 KB; with the ceilings, 43.7
   KB, and the default answer is unchanged at 21.2 KB. Each ceiling has one
   source: memories stop at the deepest context Leteo itself is ever configured
   to open with, because asking past `--context deep` asks for something no
   installation produces, and the lists with no such setting take the store's
   own ceiling for a context read.

   Both of a context's lists, not one of them. Pinned memories are listed on
   top of the budget rather than inside it — a project with as many pins as the
   budget got its pins and nothing else, and the reward for deciding what
   matters must not be to stop being told what happened — but on top of a bound
   is not outside every bound, and that is where the pins were. With 360 of
   them `mem_context` answered 370 memories in 229.5 KB while the ceiling of 80
   was in force on the other list, and the opening block, which takes no limit
   from anyone, carried the same 370 into every session start. Each list has its
   own ceiling now and neither starves the other: 90 memories and 55.4 KB
   through the tool, 12.6 KB in the block.

   And "its own ceiling" is the one the caller asked for, which is what that
   sentence claimed and the code did not do. The pinned half took
   `ContextSize::Deep` — the deepest anybody is ever configured to open with —
   while the budget beside it is whatever the caller or the `context_size`
   setting passed, so the two matched only on `deep`. Somebody who chose `slim`,
   whose whole purpose is a small opening, got twenty recent memories and eighty
   pinned ones. Driven against a copy of a real store with a hundred pins:
   `slim` answered with a hundred memories and 75 KB, `mem_context` asked for
   five answered with eighty-five and 73 KB, and the three sizes produced a
   block of exactly the same length as each other. They are now 14.8, 31.4 and
   48.0 KB, and asking for five gives ten memories. Both doors are guarded; the
   tool's was not, and restoring the fixed ceiling there left the whole suite
   green.

   What did not fit is said rather than swallowed. A pin is the most deliberate
   thing in the store, so the tool answers `pinned_omitted` and the block prints
   a line naming the number and where to find them. That guard needed a fixture
   above the ceiling to mean anything: with thirty pins nothing is cut and the
   assertion passes without watching a thing.

   And a count is not a size. A memory count and per-line lengths bound how many
   entries there are and how long each is, not how large the answer grows, so a
   store whose rows are long outgrew every ceiling it had. Both surfaces that
   open a context — `mem_context` and the session-start block — now carry a byte
   budget from `ContextSize::bytes()`, one source: 15,000, 32,000 and 49,000
   bytes for `slim`, `full` and `deep`, the measured sizes of the three blocks
   (14.8, 31.4 and 48.0 KB above) rounded up, so the ceiling sits where the
   three sizes already are and only a store that outgrows them is cut. The cut
   is at an entry boundary and never mid-line, and what it leaves out is
   counted: the block prints a line naming how many memories, prompts and
   sessions it dropped, and the tool answers `memories_omitted`,
   `prompts_omitted` and `sessions_omitted`. The newest work and the pinned
   memories are dropped last, so a budget spends the oldest and least
   informative entries first.

   And the byte budget is a parameter, `byte_limit` (also accepted as
   `max_bytes`), so a caller can ask for less than the size setting for one
   call. It only ever shrinks the answer below the size setting, never raises it:
   the setting is the budget a person chose for this store, and one call is not
   the place to overrule it. The deep ceiling — 49,000, the largest this surface
   ever produces — is what the parameter publishes as its `maximum`, the largest
   an answer reaches once the setting itself is deep. Every answer carries
   an envelope whatever it holds — the project, the language, the empty lists —
   measured at 350 bytes with the default language, and a bound under it cannot
   be met however much is dropped: the reply says so with `byte_limit_unmet`
   rather than reporting a bound it did not keep. The floor is published as the
   parameter's `minimum`, so a caller sees before the call why a smaller number
   would not be honoured; a chosen language directive is longer and moves the
   true floor, which is why the not-met answer is decided by measuring the
   reply rather than by comparing against the published number.

   And the lists nothing asks for. A budget is a parameter somebody passes;
   `mem_doctor` has none for its violations, because `PRAGMA foreign_key_check`
   answers one row per orphaned row and the tool carried every one of them: 300
   orphans made a 54.7 KB reply, scaling with the damage, so the answer was
   largest exactly when something is wrong and an agent is reading it to find
   out what. Twenty examples now, with the rest counted in
   `foreign_key_violations_omitted` — nothing is lost, because the total is
   already a sentence in `issues` and the repair is `--repair` rather than
   anything done per row. Cut at the tool and not in the store, so
   `leteo doctor` still prints the inventory: a pipe has no context window to
   spend, which is the same split `mem_context` and `leteo context` make.

   Each check in the report carries a `severity` — `error`, `warning` or
   `info` — and `healthy` is the absence of an `error`, so an agent can tell a
   degraded capability from a broken store; see
   [`store-and-schema.md`](store-and-schema.md) §4. A `check` argument runs only
   that check.

   Held over the whole surface rather than tool by tool, so the next budget
   cannot arrive without one — which is how the last two were found. Both
   `mem_search` and `mem_review` carried the same note beside their `limit`,
   that the floor is published rather than discovered, and neither published
   the other end: `mem_search` applied a ceiling readable only by asking for
   more and counting what came back, and `mem_review` had none at all. The
   reread queue is the one list where a large number is the obvious thing to
   ask for — an opening block saying 269 memories are due invites asking for
   269 — and a real store answered with all of them, 444 KB, against 33.3 KB
   now.

4. **A memory carries its own caveats.** When a later memory supersedes or
   contradicts one being handed over, the caveat travels attached to that
   memory, on every tool that returns one — including `mem_review`, which is the
   strongest case of the six: a queue that says "reread this decision" is wrong
   to stay silent about the memory that already replaced it.

5. **A write belongs to a project, and the project is detected, not asserted.**
   An explicit `project` argument is accepted only when it matches what the
   working directory detects or names a project the store already holds.
   Anything else fails with `unknown_project`; a directory holding several fails
   with `ambiguous_project` and a recovery token, and the agent is told to ask
   rather than guess.

   `ambiguous_project` on every door, including the one that mints no token.
   Two functions build that envelope — one a method that issues a token, one
   free — and which a call site reaches depends only on whether it has a
   `self`. `mem_session_start` goes through the free one and answered
   `project_detection_failed`: a code that says detection is broken for a
   directory where nothing is broken, and not the code the server instructions
   tell an agent to recognise so it can ask. It listed the candidates and hid
   what they were for.

   The remedies differ and each error carries its own. A write proves the user
   was asked, by replaying the token. `mem_session_start` takes the name
   directly — one of the candidates or a new one — because introducing a
   project is what it is for, so nothing is minted for it and nothing is
   required. A detection that genuinely failed, with no candidates to offer,
   still says `project_detection_failed`: not knowing and having a choice are
   different answers.

   And it offers every project in the directory, not the first two it found.
   The scan stopped as soon as it had two, which answered the only question it
   was asked — is this ambiguous — and became wrong when the same list turned
   into the choices an agent offers and the whitelist a replay is checked
   against. On a real workspace of 56 directories holding 27 repositories it
   offered two: the first two alphabetically, neither one anybody works in.
   Answering with the project 32nd by name came back `invalid_project_choice`,
   so from that directory a memory could be filed only under a project its
   owner does not use. Reading all 56 and asking each for a `.git` costs 0.3 ms,
   so the count that bounded the scan was protecting nothing an ordinary
   workspace does; the deadline beside it is what a pathological directory runs
   into, and it is unchanged.

   What that costs was measured rather than assumed, because the scan is now
   longer and a hook is a fresh process every time. A directory with a git root
   never scans children at all, so the ordinary case is untouched: 9 ms for a
   `user-prompt-submit` inside a repository, the same as before. The real
   ambiguous directory — 56 entries — is 10 ms, and a contrived one of 250 is
   14 ms with a worst case of 108, against a budget of five seconds. The
   envelope stays readable too: 250 directories holding 125 repositories offer
   100 of them in 4.3 KB, because the scan limit stops at 200 directories, and
   the ninetieth name in that list is accepted on replay.

   Driven end to end through the built binary against a directory holding two
   git repositories: the write is refused with the candidates, a token and the
   instructions; replaying without the token is refused; replaying with a
   project nobody offered is refused as `invalid_project_choice`; and the
   replay with both lands the memory under `user_selected_after_ambiguous_project`.

   **A scan that ran out of time says so and never passes for one that
   succeeded.** The scan has a deadline, and a truncated one used to skip the
   whole decision and fall through to the directory basename — which is a
   *successful* source, so the refusal above simply did not happen. Standing in
   a workspace of twenty-seven repositories on a machine slow enough, memories
   were filed under the name of the folder containing them, silently, and the
   protection disappeared exactly when the machine was least able to spare the
   attention to notice it had.

   The verdict and the list are now separated, because truncation costs only
   one of them. Two repositories already found is ambiguous whether or not the
   scan finished — reaching more entries cannot turn two into one — so the
   refusal stands and the warning says the list of candidates may be short,
   which matters because that list is the whitelist a replay is checked
   against. One or none found and out of time knows nothing at all: the
   basename is still the best answer, since most directories hold no
   repositories, but it comes with a warning that it is a guess, and a single
   child is not promoted on the strength of a scan that never saw whether it
   had company.

   The hook path repeats those two warnings into its outcome and no others.
   That is where the answer sticks — a session records its project and every
   call carrying that session id inherits it — and the only surface that
   carried the warning before was `mem_current_project`, which nobody calls
   when nothing looks wrong. The third thing a detection can warn about, that
   it promoted the single repository it found, is detection succeeding and is
   not repeated: shown at every session opened in such a directory, it is the
   line that teaches somebody to skip the warnings.

   **A directory whose sessions were recorded under another project does not
   silently take the name detection now resolves to.** The name is derived from
   `origin` on every call, so adding a remote to a repository named by its
   directory, renaming the remote, or pointing it at a fork changes it, and the
   next write used to be filed under the new name with nothing said — the
   memories already recorded stayed under the old one, and the two halves of the
   project could no longer see each other. Both doors into a session ask
   `recent_projects_in_directory` before returning a detected project: the write
   path's silent auto-pick and `mem_session_start`, which is the door
   `SERVER_INSTRUCTIONS` names first and whose project every later call carrying
   that session id inherits. When the sessions recorded in this exact directory
   name a project other than the one detection resolved to, the **silent** pick
   is refused with the same `ambiguous_project` error, the same candidate list —
   the recorded names first, then the detected one — and a message naming both
   and pointing at `leteo projects consolidate`. The write path's refusal mints
   the recovery token the ambiguous-directory case uses; the session door's does
   not, because `SessionStartParams` is `deny_unknown_fields` and has no
   `project_choice_reason` or `recovery_token`, and `SERVER_INSTRUCTIONS` already
   tells the agent this door takes `project=<choice>` on its own. An explicit
   `project` resolves the drift on its own, with no reason and no token, exactly
   as an explicit project resolves the session door: naming a side is the choice
   the ambiguity was asking for, and the tools that share this path cannot all
   send a token — `mem_update` carries neither a reason nor a token and
   `mem_capture_passive` carries no project at all. `mem_capture_passive`, which
   cannot name a project and has nobody to prompt, files under the directory's
   **recorded** project rather than the detected name, so a passive capture
   keeps the project whole instead of splitting it or failing; a capture that
   names an existing session instead files under that session's project, because
   a session owns its project. `mem_session_start` likewise returns a session
   that already exists unchanged, before the gate, so its published idempotency
   holds under a drift. The lookup is a scan of `sessions` alone, folded in Rust
   for the reason `same_directory` gives — including a symlinked spelling, so
   macOS's `/var/...` is found from the `/private/var/...` detection
   canonicalizes to — and it is asked only before the silent pick returns a
   detected project. What is drift-only is the prompt: a directory whose
   recorded sessions agree with detection returns the detected project exactly
   as before, with no prompt and no new fields, and the process override wins
   over the gate. The hook path cannot refuse or prompt, so it says the same
   thing as a warning — [`hooks.md`](hooks.md) §2 carries that and the measured
   cost of the lookup.

6. **A memory says which question it answers, or says nothing.** The link is a
   chain of three guesses, each with a guard: the prompt this process last
   recorded (same project and session), then the last prompt of the same
   session, then — only for a save that named no session — the last prompt of
   the same project inside a time window. A save may opt out with
   `capture_prompt: false`. Guarded by driving the orders rather than by reading
   them: a second session does not borrow the first one's question, and one
   asked two days ago is left unlinked rather than hung on the memory.

   The last two guesses belong to the store, not to this layer, because
   `leteo save` writes to the same table: it recorded no question at all while
   the tool beside it argued at length about which one was safe. Both doors read
   one rule now — see [`cli.md`](cli.md) §12. Driven through the built binary,
   both answer identically on all four cases they share: the session's own
   question, a second session that does not borrow the first's, the project's
   question inside the window, and one from two days ago left unlinked.

   And the field that opts out says what it is opting out of. `capture_prompt`
   described the first guess alone — the prompt this process recorded, same
   session and project — while the third attributes a save that names no
   session to the last question asked anywhere in the project inside the
   window, including one asked in another session. That is right for one agent
   doing two things and wrong for two agents sharing a project, it is the
   common case rather than a corner (every `mem_save` without a `session_id`,
   and 1,081 memories of 3,682 on a real store), and an agent deciding whether
   to pass `false` was deciding from a narrower picture than the behaviour. The
   window is published in the description and held against the constant, the
   way the preview length is.

7. **A save that finds a possible conflict returns candidates.** Each candidate
   carries its own `judgment_id`, and `mem_judge` records the verdict against
   it. All six verdicts are the agent's to settle, and none is put to the user:
   judging a pair is Leteo's bookkeeping rather than work anybody asked for, and
   a question about two memories the person does not remember writing spends the
   attention memory exists to save. `supersedes` is the one with teeth — it hangs
   a caveat on the other memory across all six surfaces that show one — and it is
   still the agent's, because `mem_judge` replaces a verdict wholesale, reason
   included, so a wrong one is corrected rather than lived with.

   The verdict is expected in the turn the candidate is reported, and
   `mem_save`'s own description says so. That sentence used to live only in the
   server's `instructions` block, which not every client shows: an agent reading
   the tool alone got a reply with `candidates` in it and no way to know they
   were work. A pair left
   unjudged is not deferred, it is dropped — this tool and a session opening are
   the only two places a `judgment_id` is ever handed out, so between them
   nothing raises the pair again. Which is why the second of those exists: see
   [`hooks.md`](hooks.md) §13 for the pairs an opening hands back, oldest first.

   Which is why a candidate has to beat the ordinary match for its own query
   rather than an absolute score. bm25 grows with how many terms a query has
   and how rare they are, so no fixed floor means the same thing for a two-word
   title and a twelve-word one: 399 of 400 real saves got the full three
   proposals, and a memory about a paella came back with three questions about
   git branch flow — questions the sentence above sends to the user. The margin
   is relative to the median of what the same query matched, the way the search
   stages and the prompt hint already work.

   Chosen against a label neither side of which this finder proposed: sixty-six
   restatements of memories the store already holds, at two difficulties, where
   the right answer is known because they were built from it; and nineteen
   memories about things the project has never had anything to do with, where
   the right answer is silence. On a copy of a real store — 1,712 memories in
   the project — it keeps 95% of the rewritten restatements and 91% of the
   reworded ones, against 95% and 92% before, while the off-domain memories
   that get any proposal fall from 19 of 19 to 9, and the proposals they get
   from 57 to 17.

   It halves the noise rather than ending it, and no margin on this query will:
   a title's words always match something in a project of that size. The margin
   also needs a background to be a margin against, so it applies only when the
   sample filled up. Where every match is as good as every other — a small
   store holding a few revisions of one memory — the median sits on top of the
   best one and the absolute floor is the whole gate, as it was everywhere
   before.

   Every refusal carries its own remedy, and the one that has none says whose
   it is. A busy store says to call again in a moment; a replay without its
   token names the token; an unknown project names the ones that exist; a
   session belonging elsewhere names both projects. `store_unavailable` is the
   only kind where the remedy is not the caller's at all — something panicked
   while holding the store, so every call after it fails the same way for as
   long as the process lives — and it said "the Leteo store lock is poisoned",
   which is the state in Rust's words. An agent reading that retries, gets it
   again, and reports that memory is broken or empty. It now says the server
   has to be restarted and that retrying will not help.

   There are three ceilings on this surface and only three: a list of rows stops
   at the store's own ceiling for a context read, the depth of a context stops
   at what `--context deep` gives, and the bytes of a context stop at
   `ContextSize::Deep.bytes()`. Each is applied from one place. Published, they
   are eight hand-written numbers in `schemars` annotations, which cannot read a
   constant — so nothing tied the two sides together, and a one-line change to
   an applied ceiling would leave the schemas publishing a limit this server no
   longer has. A guard now reads every ceiling the tools publish and requires it
   to be one of the three, and holds the doctor's example count to the same
   number its own comment claims it uses.

   Driven over the protocol, all eight are applied as published: asked for ten
   times each ceiling, `mem_context` answers with 80 memories, 20 prompts and
   the sessions it has, `mem_search` with 20, `mem_timeline` with 20 either
   side, and `mem_context` with `byte_limit: 49000` is bounded by the deep
   ceiling's bytes.

   And a page says how much of the queue it is not. The session opening names
   the whole review queue — "eighteen memories to read again, open it with
   `mem_review`" — and sends the agent to a tool that answers with its own page,
   ten by default. Driven end to end against a copy of a real store: the block
   said eighteen, the tool said ten, and nothing in the reply mentioned the
   other eight, so an agent that marks the ten reviewed has emptied a queue that
   is not empty. `due_omitted` carries the rest, counted from the same function
   the block reads, so the two cannot disagree. It is not folded into `count`,
   which means the length of `observations` and nothing else.

8. **A hint explains an answer the caller did not expect.** No match, a partial
   match, a summary saved without a name, nothing extracted from a passive
   capture — each has one sentence saying what happened and what to do about it.

   An empty answer from a read the directory narrowed says which of its two
   reasons it is: the store has never heard of this, or it is filed in another
   project. The count behind it stops at 100 and says "100 or more" when it
   does — `project <> ?` is not a range, so an exact count reads every live row
   and would make the empty answer the expensive one. `mem_search` and `mem_context` both answer this way and share the
   sentence — see [`search.md`](search.md) §4. `mem_context` matters most: every
   instruction file Leteo writes tells the agent to call it before acting, and
    for the eleven clients of fifteen that run no hooks it is the first thing they
   read.

   A search that read a term as another word says so too, naming every
   substitution — "searched for X instead of Y" — because its rows match words
   other than the ones typed and an agent that does not read the substitution
   would take a typo's answer for an exact one. The sentence is built once, in
   `corrected_terms_hint`, and `leteo search` prints the same one on stderr. See
   [`search.md`](search.md) §13.

   And one that found a memory by meaning says that: the result carries
   `semantic: true`, the page carries `SEMANTIC_MATCH_HINT` — a result so marked
   may contain none of the words asked for — and `leteo search` prints the same
   sentence. It sits after the correction sentence and before the relaxed-answer
   one, because a page the semantic stage touched is weaker than a `partial` one
   and the one sentence should name the weaker claim. See [`search.md`](search.md)
   §15.

9. **A number is named after the question it answers.** `mem_timeline` reports
   `before_total` and `after_total` — how much of the session lies on each side
   of the focus — because `before` and `after` are capped by the window asked
   for and a full list looks like an exhausted one
   ([`search.md`](search.md) §5). They replaced a single `total_in_range` that
   held the whole session's count: 221 on a real store, for every focus,
   whatever window was asked for.

   The general rule, guarded rather than remembered: a field called `count`
   carries a description saying what it counts. A list is the same question in
   another shape — `mem_stats` answers `projects` with one entry per project that
   holds a memory, most recently written first, each carrying its live-memory,
   session and prompt counts and the newest instant anything happened in it. The
   list is bounded by the store's list ceiling and `projects_omitted` counts what
   the ceiling left out, so a bounded list is not read as the whole inventory;
   `mem_doctor` answers one project's detail. `mem_context` answers `count:
   50` with five in `observations` and forty-five in `also_remembered`, and the
   description is the only place an agent can read that.

10. **An error names its own remedy where one exists.** A busy store answers
   `store_busy` and says to retry, rather than failing as though the request
   were malformed — one sentence, shared with the hooks and the command line.
   A refused relation verb lists the six that are accepted, the way `doctor`
   refusing a check code has always listed the valid ones. A refused
   cross-project relation names both projects, because a caller holding two
   opaque sync ids cannot otherwise tell which end was the odd one.

   And a soft-deleted memory answers `observation_deleted` with the date it went
   rather than `observation_not_found`, which is what an id that never existed
   gets. `mem_get_observation` had been handing the same id back with
   `state: "deleted"` all along, so the store knew the difference and said it on
   one door out of six. An agent holding an id from a caveat, an earlier search,
   or a sentence somebody typed reads "not found" as its own mistake and stops
   asking, when the body is still there to read.

   The six are one list and every sentence about them is held to it: the two
   descriptions an agent reads before choosing a verdict, and the skill that
   names all six as the agent's own to settle. The list
   was walked in the check and in its test, and copied out by hand in the three
   places somebody actually reads.

   And a query the store will not tokenise carries its own code rather than the
   one an empty query gets. An over-long query answers `query_too_long` and
   names the size it saw and the cap, where `invalid_search` is the empty query:
   two different mistakes with different remedies — type something, or trim what
   you typed — and an agent reading only the code has to be able to tell them
   apart. The cap, its unit and the measurement behind it are in
   [`search.md`](search.md) §14.

11. **A description earns its bytes.** `tools/list` is what every agent reads
    before it can do anything, and it is the largest fixed cost Leteo imposes.
    Output schemas are 28,069 bytes of it and input schemas 12,780 (59% and
    27%), and descriptions are 5,050 of the output half, 18% of it. When this
    requirement was written, two thirds of the output descriptions of the day
    were the same sentences again — a surface older than either capture here, so
    no command below reaches it — because the memory type is embedded in eight
    tools. That embedding has not changed: four descriptions still ship eight
    times each anywhere on this surface, for the memory's state, its prompt, its
    relations and what the graph says about it. `Absent unless pinned.`, which
    this requirement used to name as the repeated one, ships zero times.

    What that leaves, measured across the profile an agent is actually given —
    a release build driven over stdio with `LETEO_DATA_DIR` in a temporary
    directory, `tools/list` captured as `tl.json`. Every size here is in
    bytes, from `utf8bytelength`, and every measured figure has its command:

    ```sh
    # the capture. The three messages are written out rather than described,
    # because the grep looks for the id this file gives tools/list and a
    # reconstruction that numbers them differently leaves tl.json empty.
    # printf rather than a heredoc: a heredoc terminator has to sit at column
    # 0, and a column-0 line closes the list item that this requirement is,
    # taking the fence and everything below it out of the rendered requirement
    printf '%s\n' \
      '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"c","version":"0"}}}' \
      '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
      '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' > rpc.txt
    LETEO_DATA_DIR="$(mktemp -d)" ./target/release/leteo mcp --tools=agent \
      < rpc.txt | grep '"id":2' > tl.json

    # an empty tl.json answers everything below with a blank or a 0 rather
    # than an error, so it is checked here instead of trusted
    jq -e '.result.tools | length == 19' tl.json

    # 19 tools, 47781 bytes of array
    jq '.result.tools | length' tl.json
    jq '.result.tools | tojson | utf8bytelength' tl.json

    # the two halves: 28069 output schemas, 12780 input schemas
    jq '[.result.tools[] | .outputSchema | select(. != null)
         | tojson | utf8bytelength] | add' tl.json
    jq '[.result.tools[] | .inputSchema | select(. != null)
         | tojson | utf8bytelength] | add // 0' tl.json

    # descriptions: 3053 tool blurb, 5631 input fields, 5050 output fields
    jq '[.result.tools[]
         | (.description // "" | utf8bytelength)] | add' tl.json
    jq '[.result.tools[] | [.inputSchema | .. | objects
         | .description? | strings | utf8bytelength] | add // 0] | add' tl.json
    jq '[.result.tools[] | [.outputSchema // {} | .. | objects
         | .description? | strings | utf8bytelength] | add // 0] | add' tl.json

    # the shape keywords, each charged its value, key, quotes and colon:
    # 9644 type, 2128 $schema, 1094 $ref. A path test rather than has(),
    # because 16 of this surface's own fields are named `type`
    for k in type '$schema' '$ref'; do jq --arg k "$k" '[.result.tools[]
      | paths as $p | select($p[-1] == $k
          and (($p | length) < 2 or $p[-2] != "properties"))
      | (getpath($p) | tojson | utf8bytelength)
        + ($k | utf8bytelength) + 3] | add // 0' tl.json; done

    # what that path test excludes and what it keeps: 16 fields named
    # `type` weighing 1258, against 544 keywords of which 155 hold an array
    jq '[.result.tools[] | paths as $p | select($p[-1] == "type"
         and ($p | length) > 1 and $p[-2] == "properties")
       | (getpath($p) | tojson | utf8bytelength) + 7]
       | [length, (add // 0)]' tl.json
    jq '[.result.tools[] | paths as $p | select($p[-1] == "type"
         and (($p | length) < 2 or $p[-2] != "properties"))
       | getpath($p)] | [length, ([.[] | arrays] | length)]' tl.json

    # the 4 descriptions that ship eight times each anywhere on the surface,
    # printed rather than named, and the 0 of the sentence this text dropped
    jq -r '[.result.tools[] | .. | objects | .description? | strings]
       | group_by(.) | map(select(length == 8)) | map(.[0]) | .[]' tl.json
    jq '[.result.tools[] | .. | objects | .description? | strings
         | select(test("Absent unless pinned"))] | length' tl.json

    # the 20 bytes by which these descriptions exceed their own character
    # count, and the ten em dashes that are all of it: [[8212,10]]
    jq '[.result.tools[] | .. | objects | .description? | strings
         | (utf8bytelength - length)] | add' tl.json
    jq -c '[.result.tools[] | .. | objects | .description? | strings]
       | join("") | explode | map(select(. > 127))
       | group_by(.) | map([.[0], length])' tl.json

    # 13734 again, over every description wherever it sits in a tool, which
    # is what makes the three above the whole of it
    jq '[.result.tools[] | .. | objects | .description? | strings
         | utf8bytelength] | add' tl.json

    # 35 unique output descriptions; 20 of them name an absence, by hand
    jq -r '[.result.tools[] | [.outputSchema // {} | .. | objects
         | .description? | strings]] | flatten | unique | length' tl.json
    ```

    Every measured figure above has a command under it, because a figure nobody
    can reproduce stops being a budget. The 20-of-35 classification is the one
    exception and says so: it is a judgement, re-derivable by anyone applying
    the same question to the same 35 strings. The tool blurb, the input fields
    and the output fields sum to 13,734, which is every description on the
    surface.

    Four traps this block fell into before it was right, each worth the few
    words that avoid it.

    - `jq -c ... | wc -c` counts the newline jq prints after the array, and
      two bytes rather than one on a shell that rewrites it — so the figure
      read 47,783 and was platform-dependent besides.
      `tojson | utf8bytelength` never leaves jq, and answers 47,781.
    - `length` counts characters, `utf8bytelength` counts bytes. Across these
      descriptions the two differ by 20, which is ten em dashes. Every size
      here is the second.
    - The keyword loop tested `type` for a string value and charged nothing
      otherwise, which silently dropped the 155 whose value is an array like
      `["string","null"]`, publishing 5,919.
    - Correcting that with `has("type")` then charged 16 things that are not
      the keyword at all: this surface has fields *named* `type`, and an
      object of `properties` holding one answers `has("type")` exactly as a
      schema does. In bytes that loop totals 10,902, of which those 16 fields
      are 1,258; it published 10,900 because it was still counting
      characters, which is the trap above wearing the other one's clothes.
      The keyword weighs 9,644, and only a test on the path can tell a
      keyword from a field that happens to share its name.

    Against those, §11's five superseded figures. Measured on a capture kept
    from before #94 — 50,295 of array and 16,248 of descriptions — the array's
    48,339 was 1,956 below the surface when #95 was filed and is 558 above it
    now; the descriptions' 13,159 was 3,089 below and is 575 below. Both span
    the same 2,514 — the array crosses it, the descriptions figure stays under
    it — and 2,514 is what #94 took off in output descriptions an hour before
    this was written. The gap narrowed because the surface moved toward the
    numbers, not because anyone corrected them, and they had been quoted outside
    this repository by then. The other three have no such account: 8,274 of
    `type` against 9,644, 2,436 of `$schema` against 2,128 and 1,162 of `$ref`
    against 1,094, with no change on record that moved any of them. `type` in
    particular is 9,644 on the older capture too, because #94 removed
    description text and no keyword. Those three readings of the vanished build
    (50,295, 16,248 and that 9,644) are the only figures here no command above
    can answer, and they say so rather than being left to look reproducible.

    **The three-way split is the comparable one.** A server that ships no
    output schemas can only be held against the tool blurb and the input
    fields summed, 8,684, so reporting descriptions as a single 13,734 invites
    a comparison nobody can make. The dialect line is the only pure ceremony
    left among the keywords and it stays: dropping a *standard* keyword to
    save tokens is the opposite of the two fixes that got here — a
    non-standard `format` and a missing field, both found by clients that
    validate strictly.

    An *input* description prevents a wrong call. An *output* description
    describes something the agent is about to see, so it travels only where the
    value cannot say it — and the case that keeps recurring is an **absence**.
    A null, an empty list, a field that is simply not there: an absence
    serialises identically to a negative and means the opposite, so the schema
    is the only place to say which it is. That is `AGENTS.md`'s *say what
    could not be done* — an empty answer, a busy store, a check that could
    not run, each says which it is — carried across from the values to the
    schema that describes them.

    Counted rather than asserted: of the 35 unique output descriptions shipping
    today, 20 name an absence. Four of them verbatim, because a description
    quoted in part is not one of the 35:

    - `` `active`, `needs_review` or `deleted`. Absent when active. ``
    - `Present only when a project was explicitly requested and matched.`
    - `Carried when nothing matched, or when only some of the words did.`
    - `Why nothing came out, when nothing did.`

    The other 15 are not one case. Most are the ones named before this one and
    still good: a closed vocabulary that appears nowhere else in the schema (a
    caveat's `relation`), a number whose name has been misread (`count` is the
    length of the list, not the number that matched), an opaque name
    (`also_remembered`), a body this surface cut. Several more say what a field
    is *for* rather than when it is there, which is a case this requirement
    does not try to close. Absence is the largest single case, not the only
    one, and calling the other 15 a closed list would put in the requirement
    exactly the unmeasured claim it exists to refuse.

    What the field name already says stays in the source as a comment for
    whoever maintains it. Both halves are guarded — dropping a description that
    carried meaning is the same defect as shipping one that carries none.

    A published description is the **first paragraph** of its `///` block, on
    one line: `summary_of` cuts at the first blank line and collapses the
    whitespace. Everything below that blank line is compiled into the binary and
    dropped before `tools/list`, which is where a failure history belongs and
    where most of them already are.

    Shipping one that carries none is what the second of those guards now asks
    about. A published description carries no crate-implementation word —
    `serde`, `schemars`, `Option`, `String`, `struct`, `enum`, `impl`, `trait`,
    `Vec` — matched whole rather than as a substring, because `Option` is
    inside "Optional" twelve times over on this surface. It carries no `#[`
    either, matched as a substring because it holds no word to match. A
    published *field* description weighs no more than 400 bytes, measured
    against the 340 the longest legitimate one weighs and the 478 that shipped
    six times a session through a guard checking only that the text was one
    line with single spaces.

12. **An annotation is a claim about what the tool does, and it is driven.**
    Every tool declares `read_only_hint`, `destructive_hint` and
    `idempotent_hint`, and a client decides whether to ask its user from them.
    `mem_update` and `mem_save` both replace stored text — `mem_update` always,
    `mem_save` whenever the `topic_key` names a memory that exists — and both
    declared they made only additive updates, while `mem_delete`, which writes a
    tombstone and can be undone, was the only one warning about destruction.

    The table saying which tools are destructive was guarded against the
    declarations and against nothing else. It is driven now: each write is
    called on a memory whose body is known, and a tool that leaves that body
    unfindable has to say so. What a revision destroys is
    [an open proposal](../changes/a-revision-nobody-can-undo.md).

13. **Zero leaves a section out, and every floor is published rather than
    discovered.** A reply made of parts — `mem_context`'s memories, sessions and
    prompts, `mem_timeline`'s window either side of the focus — takes zero for
    any of them and sends none of that part. A list's own page size does not:
    zero there is a page with nothing on it, so `mem_search` and `mem_review`
    publish the floor of one they apply.

    Seven parameters used to publish `minimum: 0` and six of them handed back a
    row anyway — `schemars` derives that zero from `usize`, the way it derived
    the `format: uint` that made strict clients reject every tool. A caller who
    asked for no sessions, no prompts and no memories got one of each.

    It is a bound worth having. Leaving the sessions and the prompts out takes
    `mem_context` from 14,401 bytes to 10,921 on a real project, and until now
    there was no way to ask.

    And a floor that is not zero: `mem_context`'s `byte_limit` publishes the
    envelope every answer carries — 350 bytes with the default language — as its
    `minimum`, because a bound under it cannot be met and the reply says so with
    `byte_limit_unmet`. It is a lower bound rather than a clamp: a value below
    it is accepted and reported as unmet, not silently raised to the floor.

    The other end of the same budget: `mem_timeline` had no ceiling at all, and
    a window of a million came back with the whole session — 191 KB on a real
    one, from the tool on the surface whose purpose says a payload that pushes
    the useful part out of a context window has failed. It is bounded at the
    store's maximum for a context read, the schema publishes that maximum, and
    `before_total` and `after_total` already say how much lies beyond it.

14. **A revision the server accepts is a revision it answers.** `initialize`
    is answered with the revision the client asked for when rmcp knows it,
    and with `2025-11-25` — rmcp's real ceiling — when it does not; a client
    that asks for `2026-07-28` is answered `2026-07-28`. From that revision
    on, SEP-2549 makes `ttlMs` and `cacheScope` required on the one cacheable
    result Leteo publishes, `tools/list`. A session that negotiated it gets
    `ttlMs: 300000` and `cacheScope: "public"` — the list depends on the
    `--tools` flag the process started with, never on who is asking, and five
    minutes bounds how long a client would keep serving a list from a process
    that has since restarted with a different flag. On every revision below
    `2026-07-28` — `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05` —
    the fields are absent rather than set-and-stripped: they did not exist
    there, and publishing them would bet every legacy client on tolerating a
    property its schema never named.

    The fields are set by hand because the `#[tool_handler]` macro left no
    seam: its expansion filled both with `None`, which never serialises, so a
    client that negotiated the revision rmcp itself accepted got zero tools
    and a failed connection — ZCode 0.16.5, measured, retries in a loop. rmcp
    strips `resultType` for older peers but fills nothing for newer ones, so
    the value is the server's to state. Guarded over the wire against the
    built binary at `2026-07-28`, at every revision rmcp knows below it, and
    at the fallback an unknown version receives, with the list itself held
    byte-identical between a new-revision session and a legacy one, and the
    guard confirmed by removing each setter and watching it fail.

15. **The server instructions fit the client that delivers them.** The
    `instructions` block is what an agent reads before it calls anything, and
    Claude Code cuts it at 2,048 characters, appending `… [truncated]`. The
    block rendered to 2,246, so every session silently lost the whole SUMMARIES
    paragraph and the tail of CONFLICTS — the rules that keep a summary
    findable. It is now shorter than the bound with margin and still states
    every rule it carried, and `SERVER_INSTRUCTIONS_LIMIT` in `src/mcp/mod.rs`
    is the one constant the test counts against, so lengthening the text past
    the bound fails rather than truncating again.

16. **A mutating tool takes the project it is acting on, and refuses when the
    memory is elsewhere.** `mem_update`, `mem_delete`, `mem_pin` and `mem_unpin`
    all take `expected_project`, required by the schema. The store compares it
    to the memory's stored project inside the same write transaction as the
    change, and answers `project_mismatch` when they differ — naming both,
    because either the id was wrong or the memory is filed somewhere unexpected,
    and only the caller can tell which. A refusal changes nothing: no revision
    count, no tombstone, no sync mutation.

    Ids are not private to a project. `mem_search` with `all_projects` and the
    elsewhere-count retry both hand out ids from other projects, so an agent
    working in one repository can hold another repository's id and revise, pin
    or hard-delete it by mistake — and `mem_delete` with `hard_delete` made that
    irreversible. The check follows Engram v3.0.0, which made the same parameter
    mandatory.

    `mem_update`'s `project` is still the move target, so moving a memory takes
    both ends: `expected_project` says where it is now and `project` says where
    it goes. A memory that stays put needs only the one.

17. **A write naming an ended session is refused, and the stop hook keeps the
    summary it finds.** The `SessionStop` hook closes a session, and a write that
    still names it — `mem_save`, `mem_save_prompt`, `mem_session_summary` or
    `mem_capture_passive` — answers `session_already_ended` rather than filing
    the memory under a conversation that has closed; a save that names no session
    keeps landing in the project's manual session, and that unnamed path never
    reads the ended state, so it cannot be refused. The hook ends every session
    with no summary, and that end preserves the summary `mem_session_end` already
    wrote instead of wiping it — the replicated path had kept it since it was
    written, and this is the sibling that did not. (`mem_session_summary` writes
    a `session_summary` observation, not the session row, so it was never the one
    at risk.) Both write doors refuse alike: the CLI resolves its session through
    the same rule, so `leteo save --session` cannot file under a closed session
    either.

18. **A body over the storage bound is stored cut, and the tool says so.** Six
    write tools report it: `mem_save`, `mem_update`, `mem_save_prompt`,
    `mem_session_end`, `mem_session_summary` and `mem_judge`, each answering with
    a storage-cut report when `max_observation_length` cut what it stored. The
    report is `original_bytes`, the length the bound saw, and `stored_bytes`,
    what was kept — named `storage_truncation` on the save, update, prompt and
    session tools, and `reason_storage_truncation` and
    `evidence_storage_truncation` on a judgment, which stores two texts. It is a
    field of its own and not the preview flag beside it, because they are two
    different cuts: `content_truncated` and its siblings describe the 400-byte
    reply, and a body can come back whole and still be stored short — which is
    exactly the case where the caller has to split the memory, and the case the
    preview flag invites reading as untruncated. The length is taken after
    redaction, so a body whose bulk was a `<private>` span is stored whole and
    reports nothing. `stored_bytes` counts the marker `truncate_content`
    appends, so the caller's own bytes dropped are the difference plus that
    marker. The field is absent when nothing was cut, and the bound it reports is
    the one the store applied, read from the store rather than restated. The
    same bound governs `mem_capture_passive`, which stores each extracted
    learning through the same path, and the CLI write doors; neither reports it.

19. **A merge replaces several memories with one and hides the sources.**
    `mem_consolidate` inserts one replacement and records a judged `supersedes`
    relation from it to every source, all in one transaction: a source in
    another project, or one this store does not hold, refuses before the
    replacement row or any relation is written. Nothing is deleted — the
    relation names both ends, so the merge is traceable and reversible, and a
    re-verdict restores a source to search and context
    ([`memory-model.md`](memory-model.md) §13). The replacement lands in the
    project the sources are asserted to be in, so a merge cannot split the
    family it just joined. `source_ids` takes at least one id and refuses a
    repeat; the reply carries the new memory, its relations, and the
    `source_ids` it replaced.

20. **A version-changing write says so, and a read can ask for the versions.**
    `mem_save` and `mem_update` answer `replaced_bytes`, the size of the body
    they overwrote, whenever they changed one; a replacement under half that
    size also carries a `hint` naming `mem_get_observation` with
    `include_history`. `include_history` is a new `mem_get_observation`
    parameter, default false, and when true the reply carries `versions`: the
    titles and bodies earlier writes replaced, newest revision first, each the
    previous text whole rather than previewed. The threshold and the retention
    bound are both one constant ([`memory-model.md`](memory-model.md) §14); the
    store decides whether a write shrank and both surfaces read the decision,
    so the tool and the command line cannot disagree about what counts as one.
    Restoring is an ordinary `mem_update` with the old text; there is no restore
    tool.

    A find/replace is a content-changing write like any other, so it is inside
    that same rule: `mem_update` with `find` and `replace` edits one span of the
    stored body and keeps the text it replaced as a version, exactly as a
    whole-body write does. The tool never holds the edited text, so the store
    reports the storage cut it made from that text — see §22.

21. **`mem_search` can answer by meaning, and its description says so in a
    sentence.** When the words find little or nothing the tool goes on to the
    semantic stage ([`search.md`](search.md) §15) unless the `semantic_search`
    setting is `false`, and a result the stage added carries `semantic: true`.
    The field is optional in the declared schema, like `partial`, for the reason
    §2 gives. What this cost on `tools/list`, counted over the schemas of a
    release build the way §11 counts them: the `mem_search` blurb went from 263
    to 382 bytes and the output field descriptions from 10,479 to 10,581, so the
    array from 66,931 to 67,199 bytes. The blurb is published schema text, which
    is why it is one sentence and says what the field means only by naming it.

    A query may narrow to `type: session_summary`. That is the one way a summary
    is found, because every relaxed stage leaves summaries out otherwise
    ([`search.md`](search.md) §6), and the schema names the type so a caller can
    ask for one.

    `mem_search` keeps `read_only_hint: true`, and that stays a true statement
    about memories: the stage may write rows into `observation_vectors`, a cache
    derived from them that no tool returns, replicates or counts
    ([`store-and-schema.md`](store-and-schema.md) §16), and a call repeated
    returns the same answer. It can also be the first call to a store that does
    the work — see [`search.md`](search.md) §15 for what that costs — so the
    write is a reason to turn the setting off for a store that must stay
    byte-identical under reads, such as one on read-only media, which still
    answers, from vectors made for the question and not kept.

22. **A body can be edited in place, and a context answer can be bounded in
    bytes.** `mem_update` takes `find` and `replace` as an alternative to
    `content`: the stored body is read and edited inside the same write
    transaction, `find` is counted against it, and the edit is refused unless it
    names exactly one span — `edit_not_found` when the text is not there and
    `edit_ambiguous` when it is there more than once, each changing nothing. The
    replacement then goes through the same redaction and storage bound a
    whole-body write does, so a `<private>` span cannot arrive by the partial
    door. This covers the body and not the title: a title is one line, and a
    span of one is not a unit anybody means. `mem_context` takes `byte_limit`
    (also accepted as `max_bytes`), a per-call ceiling that only ever shrinks
    the answer below the size setting; the answer is cut at an entry boundary
    as §3 describes, and a bound under the envelope every answer carries is
    reported with `byte_limit_unmet` rather than missed. History paging is
    deliberately absent: `include_history`
    returns the versions the retention bound keeps, and nothing walks further
    back.

## Invariants

- Titles printed into anything an agent reads are folded to a single line and
  cut to 140 characters. A newline in a title once injected a fabricated memory
  into the opening context; the project name had the same hole for the same
  reason.
- Any limit published in a reply is the limit that did the cutting.
- A tool that hands back a memory hands back its caveats. This is checked across
  the whole set, not per tool — four of six once carried them.

## Where it lives

- `src/mcp/mod.rs` — the server envelope: negotiation, the tool list and its cache fields
- `src/mcp/tools.rs` — the tool router and every handler
- `src/mcp/output.rs` — the typed replies, the previews, the hints
- `src/mcp/params.rs` — parameter parsing and the project gate
- `src/mcp/tests.rs` — §5's drift refusal is held by
  `a_remote_that_changed_makes_the_write_ask_which_project`, its session door by
  `a_remote_that_changed_makes_the_session_door_ask_too`, its agreeing half by
  `a_directory_whose_sessions_agree_is_not_asked_which_project`, its
  explicit-project half by `a_drifted_directory_lets_mem_update_move_a_memory`,
  its passive-capture half by
  `a_passive_capture_in_a_drifted_directory_keeps_the_project_whole`, its
  named-session half by
  `a_passive_capture_naming_a_session_files_under_that_sessions_project`, and
  the session door's idempotency under a drift by
  `a_repeated_session_start_in_a_drifted_directory_returns_the_session_unchanged`
- `src/store/tests/versions.rs` — §22's edit is held by
  `a_find_and_replace_keeps_the_version_it_replaced`,
  `a_find_that_occurs_twice_changes_nothing`,
  `a_find_that_is_absent_changes_nothing`,
  `a_private_span_written_through_replace_is_not_stored` and
  `a_find_and_replace_reaches_the_peer_as_an_upsert`, and the byte bound by
  `a_replace_that_outgrows_the_bound_reports_the_cut`,
  `mem_context_honours_a_byte_limit_below_the_size` and
  `mem_context_says_when_a_byte_limit_cannot_be_met` in `src/mcp/tests.rs`
- `src/store/sessions.rs` — `recent_projects_in_directory`, the lookup the
  write path and the session-start hook share
- `src/store/tests/diagnostics.rs` — the store-side project counts and the bound `mem_stats` applies
- `tests/mcp_protocol.rs` — the wire surface, driven through the built binary

## Related

- [`memory-model.md`](memory-model.md) — what these tools write
- [`search.md`](search.md) — what `mem_search` does with a query
- [`hooks.md`](hooks.md) — the same store, unattended
- [`cli.md`](cli.md) — the same operations for a person
