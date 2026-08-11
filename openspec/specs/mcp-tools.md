# MCP tools

## Purpose

The surface agents actually use: an MCP server over stdio exposing the store as
typed tools. Everything here is answered in one round-trip and read by a model,
so the cost of a reply is part of its correctness — a payload that pushes the
useful part out of a context window has failed even if every field is right.

## Behaviour

1. **Twenty-two tools, one store, and `--tools` names which of them.** The
   profiles are `agent`, `admin` and `all`, and a single tool may be named
   instead. Anything else is refused at start-up with the profiles and the
   twenty-two spelled out: an unknown name used to be kept as though it were a
   tool, so it matched nothing and every route was removed. `--tools=agnet`
   started a memory server with no memory tools on it, in silence, and
   `--tools=AGENT` did the same — and what an agent sees then is "Leteo's tools
   are missing", which the skill answers with "run `leteo setup` and restart".
   A typo sending somebody to reinstall an install that was fine.

   Twenty-two tools. Writing: `mem_save`, `mem_update`,
   `mem_delete`, `mem_save_prompt`, `mem_session_start`, `mem_session_end`,
   `mem_session_summary`, `mem_capture_passive`, `mem_pin`, `mem_unpin`,
   `mem_judge`, `mem_merge_projects`. Reading: `mem_search`, `mem_context`,
   `mem_get_observation`, `mem_timeline`, `mem_review`, `mem_compare`,
   `mem_stats`, `mem_doctor`, `mem_current_project`, `mem_suggest_topic_key`.

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

   This covers what a caller has just sent as much as what it is being shown.
   `mem_update` echoed a memory whole — 4,556 bytes to change a title, byte for
   byte what `mem_get_observation` sends — `mem_save_prompt` echoed the prompt,
   which is where people paste, and `mem_judge` echoed both the reason and the
   evidence: 24,359 bytes of the caller's own words to record one verdict.
   `mem_timeline` previewed its neighbours and handed its focus over whole,
   which its own description already denied. Nine tools preview; two guards hold
   the rule — one names them and one gives every surface twenty thousand bytes
   and requires a small answer, which is what found the ninth.

   That second guard measures what an agent receives, which is the answer
   twice. A reply carrying `structuredContent` also carries the same JSON
   serialised into a text block, because the protocol asks for one so that a
   client predating structured output still gets an answer — so a 20-result
   `mem_search` on a real store is 16,428 bytes of structured content and
   16,957 of the same thing as text, 33,385 in all. The guard used to weigh the
   struct alone, which held every surface to half the number it costs. The bar
   has not moved; what changed is that the largest surface now sits at 5,858
   bytes of the 8,000 allowed where it used to sit at half that.

   Whether to stop sending the text half to clients that negotiate a protocol
   version which has structured output is open, and deliberately not decided
   here: it would halve every reply, the protocol says SHOULD rather than MUST,
   and Leteo configures twelve clients whose behaviour without it has not been
   measured.

   And so are the lists, at both ends. Every budget a tool takes has a ceiling
   and publishes it in its own schema, so a caller can plan against it: a
   ceiling that lives only in the code is one nobody can read. `mem_timeline`
   was given one after a window of a million came back with a whole session,
   191 KB, and `mem_context` — the only route to context nine of the twelve
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

   There are two ceilings on this surface and only two: a list of rows stops at
   the store's own ceiling for a context read, and the depth of a context stops
   at what `--context deep` gives. Each is applied from one place. Published,
   they are seven hand-written numbers in `schemars` annotations, which cannot
   read a constant — so nothing tied the two sides together, and a one-line
   change to either applied ceiling would leave six schemas publishing a limit
   this server no longer has. A guard now reads every ceiling the tools publish
   and requires it to be one of the two, and holds the doctor's example count to
   the same number its own comment claims it uses.

   Driven over the protocol, all seven are applied as published: asked for ten
   times each ceiling, `mem_context` answers with 80 memories, 20 prompts and
   the sessions it has, `mem_search` with 20, `mem_timeline` with 20 either
   side.

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
   capture, a type outside the eight — each has one sentence saying what
   happened and what to do about it.

   An empty answer from a read the directory narrowed says which of its two
   reasons it is: the store has never heard of this, or it is filed in another
   project. The count behind it stops at 100 and says "100 or more" when it
   does — `project <> ?` is not a range, so an exact count reads every live row
   and would make the empty answer the expensive one. `mem_search` and `mem_context` both answer this way and share the
   sentence — see [`search.md`](search.md) §4. `mem_context` matters most: every
   instruction file Leteo writes tells the agent to call it before acting, and
   for the nine clients of twelve that run no hooks it is the first thing they
   read.

9. **A number is named after the question it answers.** `mem_timeline` reports
   `before_total` and `after_total` — how much of the session lies on each side
   of the focus — because `before` and `after` are capped by the window asked
   for and a full list looks like an exhausted one
   ([`search.md`](search.md) §5). They replaced a single `total_in_range` that
   held the whole session's count: 221 on a real store, for every focus,
   whatever window was asked for.

   The general rule, guarded rather than remembered: a field called `count`
   carries a description saying what it counts. A list is the same question in
   another shape — `mem_stats` answers `projects` with the ones holding at
   least one memory, most recently written first, which is two short of the
   nineteen a real store knows and is the useful order rather than the
   inventory. `mem_context` answers `count:
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

11. **A description earns its bytes.** `tools/list` is what every agent reads
    before it can do anything, and it is the largest fixed cost Leteo imposes:
    62% of it is output schemas, a fifth of those were descriptions, and two
    thirds of those were the same sentences again — the memory type is embedded
    in eight tools, so `Absent unless pinned.` shipped eight times.

    What that leaves, measured across the profile an agent is actually given:
    48,339 bytes for nineteen tools, of which 13,159 are descriptions, 8,274 the
    `type` keywords that carry the shape, 2,436 the `$schema` dialect line and
    1,162 the `$ref`s that keep a repeated type from being written twice inside
    one document. The dialect line is the only pure ceremony left and it stays:
    dropping a *standard* keyword to save 610 tokens is the opposite of the two
    fixes that got here — a non-standard `format` and a missing field, both
    found by clients that validate strictly.

    An *input* description prevents a wrong call. An *output* description
    describes something the agent is about to see, so it travels only where the
    value cannot say it: a closed vocabulary that appears nowhere else in the
    schema (`state`, a caveat's `relation`), a number whose name has twice been
    misread (`count`), an opaque name (`caveats`). What the field name already
    says stays in the source as a comment for whoever maintains it. Both halves
    are guarded — dropping a description that carried meaning is the same defect
    as shipping one that carries none.

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

    The other end of the same budget: `mem_timeline` had no ceiling at all, and
    a window of a million came back with the whole session — 191 KB on a real
    one, from the tool on the surface whose purpose says a payload that pushes
    the useful part out of a context window has failed. It is bounded at the
    store's maximum for a context read, the schema publishes that maximum, and
    `before_total` and `after_total` already say how much lies beyond it.

## Invariants

- Titles printed into anything an agent reads are folded to a single line and
  cut to 140 characters. A newline in a title once injected a fabricated memory
  into the opening context; the project name had the same hole for the same
  reason.
- Any limit published in a reply is the limit that did the cutting.
- A tool that hands back a memory hands back its caveats. This is checked across
  the whole set, not per tool — four of six once carried them.

## Where it lives

- `src/mcp/tools.rs` — the tool router and every handler
- `src/mcp/output.rs` — the typed replies, the previews, the hints
- `src/mcp/params.rs` — parameter parsing and the project gate
- `src/mcp/tests.rs`

## Related

- [`memory-model.md`](memory-model.md) — what these tools write
- [`search.md`](search.md) — what `mem_search` does with a query
- [`hooks.md`](hooks.md) — the same store, unattended
- [`cli.md`](cli.md) — the same operations for a person
