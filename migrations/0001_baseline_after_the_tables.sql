-- Everything the tables above do not do, in one step, before the first release.
--
-- These were eleven migrations, written one at a time as each thing was found,
-- then ten of them folded into a file numbered 16 and one more added after it.
-- Nothing has been released, so no database outside this repository ever ran any
-- of them, and there is nothing for the append-only rule to protect: what that
-- rule protects is a *released* migration, because the databases that already
-- ran it will not run it again.
--
-- Now they are part of the baseline and the numbering starts again at 1. There
-- is one migration and it is this schema; a store either carries it or is
-- brought to it by inspection, which is what `adopt_to_baseline` above has
-- always done for a database of unknown provenance.
--
-- The one store in the world stamped higher than 1 is the one this was
-- developed against, and it is re-stamped by hand rather than by code that
-- would outlive the reason for it.
--
-- Every statement below is idempotent, and that is what makes one file safe:
-- the indexes and the virtual table are `IF NOT EXISTS`, the triggers are
-- dropped before they are created, and the statements that rewrite data all
-- fold a value onto a canonical one. Running them twice is running them once.

CREATE INDEX IF NOT EXISTS idx_obs_created_order
ON observations(datetime(created_at) DESC, id DESC);

-- ---------------------------------------------------------------------------
-- was 0008_exact_index.sql
-- ---------------------------------------------------------------------------

-- A second full-text index, of the words as they were actually written.
--
-- `observations_fts` is stemmed (`porter unicode61`), and that is what lets a
-- question asked with a different inflection find anything at all: measured on
-- a real store, a question with two of six words re-inflected is answered 63%
-- of the time here and 0% by the same store searched without a stemmer.
--
-- The cost of stemming is that more memories match the same words, so the one
-- somebody meant is diluted: quoting six words straight out of a memory finds
-- it first 78% of the time, against 84% unstemmed. Both are real, they pull in
-- opposite directions, and no single tokenizer has both — the tokenizer is a
-- property of the table.
--
-- So there are two tables and the search reads both. Merged by where each one
-- put a memory rather than by score, that is 84.3% on quoted words and 37.0%
-- on re-inflected ones, against 78.0% and 37.3% from the stemmed index alone.
--
-- What it costs, measured on the same 46 MB store: 5.1 MB of index, 228 ms to
-- build once, 0.03 ms on a search, and 0.04 ms — three per cent — on saving a
-- memory.
--
-- Written as one migration rather than added to the baseline because a new
-- database runs the migrations too: `migrate` stamps an adopted store `1` and
-- then applies everything above it, so this is the only place that needs to
-- know how the table is made.
CREATE VIRTUAL TABLE IF NOT EXISTS observations_exact USING fts5(
    title, content, tool_name, type, project, topic_key,
    content='observations', content_rowid='id',
    tokenize = 'unicode61'
);

DROP TRIGGER IF EXISTS obs_exact_insert;
DROP TRIGGER IF EXISTS obs_exact_delete;
DROP TRIGGER IF EXISTS obs_exact_update;
CREATE TRIGGER obs_exact_insert AFTER INSERT ON observations BEGIN
    INSERT INTO observations_exact(rowid, title, content, tool_name, type, project, topic_key)
    VALUES (new.id, new.title, new.content, new.tool_name, new.type, new.project, new.topic_key);
END;
CREATE TRIGGER obs_exact_delete AFTER DELETE ON observations BEGIN
    INSERT INTO observations_exact(observations_exact, rowid, title, content, tool_name, type, project, topic_key)
    VALUES ('delete', old.id, old.title, old.content, old.tool_name, old.type, old.project, old.topic_key);
END;
CREATE TRIGGER obs_exact_update AFTER UPDATE ON observations BEGIN
    INSERT INTO observations_exact(observations_exact, rowid, title, content, tool_name, type, project, topic_key)
    VALUES ('delete', old.id, old.title, old.content, old.tool_name, old.type, old.project, old.topic_key);
    INSERT INTO observations_exact(rowid, title, content, tool_name, type, project, topic_key)
    VALUES (new.id, new.title, new.content, new.tool_name, new.type, new.project, new.topic_key);
END;

-- Last, and after the triggers, so a database interrupted between the two is
-- retried rather than left with an index nothing keeps up to date.
INSERT INTO observations_exact(observations_exact) VALUES('rebuild');

-- ---------------------------------------------------------------------------
-- was 0009_orphaned_journal.sql
-- ---------------------------------------------------------------------------

-- Drops the replication journal of projects nobody replicates.
--
-- On a real store this is 9,527 rows and 15.2 MB — 29% of a 51.7 MB database —
-- for a cloud that was never configured: no `cloud.json`, nothing enrolled, and
-- every row unacknowledged. 4,954 of them are sessions, from a defect where an
-- existing session was queued again on every memory saved.
--
-- They are not a backlog. Journalling is gated on enrolment, so nothing new
-- joins them, and they cannot be sent: pushing is gated on the configured
-- project list, which enrolment mirrors. What settles it is `enroll_project` —
-- it deletes a project's unacknowledged mutations and backfills every row the
-- project holds in its current state, precisely so that a journal skipped while
-- unenrolled loses no history. So whichever way a store goes from here, these
-- rows are discarded: by staying unenrolled, or by being enrolled.
--
-- Acknowledged rows are left alone. Those are the record of what a peer already
-- has, and the retention window is what removes them.
--
-- The file does not shrink — SQLite keeps the freed pages for reuse rather than
-- returning them to the filesystem. `VACUUM` would return them, and it rewrites
-- the whole database, so that is left to whoever wants the space back today
-- rather than done inside an upgrade every agent races to run.
DELETE FROM sync_mutations
 WHERE acked_at IS NULL
   AND project NOT IN (SELECT project FROM sync_enrolled_projects);

-- ---------------------------------------------------------------------------
-- was 0010_project_ordering_index.sql
-- ---------------------------------------------------------------------------

-- The index a project's own recent memories are read through.
--
-- Every session opens by asking one question: the newest memories of this
-- project. `idx_obs_project` finds the project's rows and then the ordering is
-- a temporary B-tree over all of them, so what a session costs grows with how
-- much the project remembers — which is the wrong way round, since the answer
-- is always the same fifty.
--
-- Measured on a real store, the same query the session-start hook runs:
--
--   project      memories   before      after
--   leteo             171   1.12 ms   0.52 ms
--   task-board        542   2.32 ms   0.44 ms
--   almanac           690  11.03 ms   0.74 ms
--   ledgerly        1,711   9.20 ms   0.46 ms
--
-- The point is not the multiple, it is the shape: after this the plan is a
-- range scan of the index and there is no sort at all, so the cost stops
-- following the size of the project.
--
-- `datetime(created_at)` and not the raw column, because that is what the query
-- orders by — the store holds two timestamp formats on purpose, one from
-- adoption and one from the cloud, and ordering by the text is only the same
-- order while every row is written the same way. The same reasoning as
-- `idx_obs_created_order`, which stays: it serves the listing that names no
-- project.
CREATE INDEX IF NOT EXISTS idx_obs_project_order
ON observations(project, datetime(created_at) DESC, id DESC);

-- ---------------------------------------------------------------------------
-- was 0011_session_project_index.sql
-- ---------------------------------------------------------------------------

-- Finding a project's sessions without reading everybody else's.
--
-- `sessions` had one index, the one SQLite makes for the primary key. So the
-- question every session opening asks — the recent sessions of this project —
-- read every session ever recorded, and the store gains one per agent session
-- for ever.
--
-- Measured on a real store by padding the table, the same query the hook runs:
--
--   sessions   without    with
--        485   0.043 ms   0.004 ms
--      2,485   0.130 ms   0.004 ms
--     10,485   0.511 ms   0.004 ms
--     40,485   3.474 ms   0.004 ms
--
-- Flat against linear, which is the whole argument: today it is worth forty
-- microseconds, and that is said plainly rather than dressed up. The store it
-- was measured on collected 485 sessions in about two months.
--
-- The ordering stays a temporary B-tree and that is fine: it sorts one
-- project's sessions rather than every session there is, and the `GROUP BY`
-- would need one anyway.
--
-- Nothing for `prompts`. The same shape looked likely there and was measured
-- and dropped: the recent-distinct-prompts query is driven by its `GROUP BY
-- content` subquery, and a `(project, datetime(created_at))` index left the
-- plan untouched at 0.005 ms either way.
CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project);

-- ---------------------------------------------------------------------------
-- was 0012_fold_stale_types.sql
-- ---------------------------------------------------------------------------

-- The type vocabulary, applied to memories that were saved before it existed.
--
-- `normalize::kind` folds a handful of words onto the eight the skill teaches,
-- because the type is a filter: asking for `bugfix` runs `type = 'bugfix'`, so
-- a memory filed as `bug` is invisible to the question it answers. That fold
-- runs at the door, which means it only ever applied to memories saved after
-- it was written. Everything already in the store kept whatever word it
-- arrived with.
--
-- On a real store of 3,769 memories that left exactly eighteen, all of them
-- `manual` — which is not a description of anything. It is the default value
-- of `mem_save`'s `type`, so it is what a caller who did not choose leaves
-- behind, and no agent ever searches for it. Fifteen belong to one project and
-- three to another; the oldest is from June.
--
-- The mapping is the one in `normalize::kind`, written out. It is deliberately
-- not every word in that function: the ones that stay are the ones the store
-- actually held. A migration that folds words nobody ever wrote is a claim
-- about data that does not exist.
--
-- What this does not touch:
--
--   Types the code keeps verbatim on purpose — `implementation`, `feature`,
--   `project`, `reference`, 32 memories between them. `normalize::kind` leaves
--   an unrecognised word alone rather than forcing it into the nearest
--   documented bucket, because an honest unknown type still says something
--   true and a wrong one does not.
--
--   The review clock. None of the folded words maps to `decision`, `policy` or
--   `preference`, which are the only three that come due, so no date has to be
--   set or cleared here. If a future fold does map to one of those, it has to
--   set `review_after` too — see `Store::reschedule_review`.
--
-- The full-text indexes follow by their update triggers: `type` is an indexed
-- column in both `observations_fts` and `observations_exact`, and both carry an
-- AFTER UPDATE trigger that rewrites the row.

UPDATE observations
SET type = CASE type
        WHEN 'bug' THEN 'bugfix'
        WHEN 'fix' THEN 'bugfix'
        WHEN 'hotfix' THEN 'bugfix'
        WHEN 'incident' THEN 'bugfix'
        WHEN 'regression' THEN 'bugfix'
        WHEN 'design' THEN 'architecture'
        WHEN 'adr' THEN 'architecture'
        WHEN 'refactor' THEN 'architecture'
        WHEN 'learning' THEN 'discovery'
        WHEN 'research' THEN 'discovery'
        WHEN 'investigation' THEN 'discovery'
        WHEN 'root_cause' THEN 'discovery'
        WHEN 'root-cause' THEN 'discovery'
        WHEN 'passive' THEN 'discovery'
        WHEN 'manual' THEN 'discovery'
        WHEN 'convention' THEN 'pattern'
        WHEN 'guideline' THEN 'pattern'
        WHEN 'rule' THEN 'pattern'
        WHEN 'setup' THEN 'config'
        WHEN 'infra' THEN 'config'
        WHEN 'infrastructure' THEN 'config'
        WHEN 'ci' THEN 'config'
        WHEN 'configuration' THEN 'config'
        ELSE type
    END
WHERE type IN (
    'bug', 'fix', 'hotfix', 'incident', 'regression',
    'design', 'adr', 'refactor',
    'learning', 'research', 'investigation', 'root_cause', 'root-cause',
    'passive', 'manual',
    'convention', 'guideline', 'rule',
    'setup', 'infra', 'infrastructure', 'ci', 'configuration'
);

-- ---------------------------------------------------------------------------
-- was 0013_project_names_that_are_paths.sql
-- ---------------------------------------------------------------------------

-- Prompts and sessions filed under a directory instead of a project.
--
-- `normalize::project` reduces a path-shaped name to its last segment now, and
-- that runs at the door, which means it only ever applied to what was written
-- after it. On a real store of 3,769 memories it left 44 rows behind:
--
--   h:\repo\<a project>                            15 rows -> <a project>
--   h:\repo                                        28 rows -> repo
--   \users\<someone>\.agents\skills\task-board\     1 row  -> task-board
--
-- All three last segments are projects that store actually holds, so this is a
-- repair rather than a guess. Nothing finds those rows as they stand: every
-- read narrows by project, and the memories of those projects are filed under
-- the name, so the prompts sit in a project that exists nowhere else and are
-- out of every opening context.
--
-- Two names are left alone on purpose — a username and a two-letter word,
-- three rows — because they are not paths. They are wrong in a way no rule here can know
-- about, and inventing a project for them would be worse than leaving them.
--
-- The expression is SQLite's idiom for "what follows the last separator":
-- removing the separators leaves the set of characters the tail is made of,
-- and `rtrim` with that set eats the tail, leaving the prefix to delete. Both
-- slashes, because a name written on Windows arrives with backslashes and one
-- written anywhere else with forward ones, and this store holds both. Checked
-- against all five real values and the four that must not move.
--
-- Observations are not touched and need not be: not one of them carries a
-- path. A memory is written through a door that has always had more of its
-- normalisation than these two did.

UPDATE prompts
SET project = lower(replace(
            rtrim(replace(project, '/', '\'), '\'),
            rtrim(rtrim(replace(project, '/', '\'), '\'),
                  replace(rtrim(replace(project, '/', '\'), '\'), '\', '')),
            ''
        ))
WHERE ifnull(project, '') <> ''
  AND (project LIKE '%/%' OR project LIKE '%\%');

UPDATE sessions
SET project = lower(replace(
            rtrim(replace(project, '/', '\'), '\'),
            rtrim(rtrim(replace(project, '/', '\'), '\'),
                  replace(rtrim(replace(project, '/', '\'), '\'), '\', '')),
            ''
        ))
WHERE ifnull(project, '') <> ''
  AND (project LIKE '%/%' OR project LIKE '%\%');

-- ---------------------------------------------------------------------------
-- was 0014_review_due_index.sql
-- ---------------------------------------------------------------------------

-- Finding what is due for review without reading everything that is not.
--
-- `mem_review` asks one question — which memories are past their date — and it
-- asked it by scanning the whole table and sorting the answer in a temporary
-- B-tree. On a real store that is 3,769 rows read and ordered to find among
-- the 268 that even have a date, and today none of them are due: 4.5 ms to
-- answer with fifty bytes.
--
-- Measured on a copy of that store, padded, with the same query the tool runs:
--
--   memories   without      with
--      3,769   4.466 ms   0.002 ms
--      8,769   5.213 ms   0.011 ms
--     23,769   7.819 ms   0.006 ms
--     63,769  14.166 ms   0.006 ms
--
-- Flat against linear, which is the argument. Today it is worth four
-- milliseconds on a tool an agent calls now and then; the reason to add it is
-- the shape of the second column rather than the size of the first.
--
-- Two things make it small. It indexes `datetime(review_after)` rather than
-- the column, because that is the expression the query filters and orders by
-- and SQLite will not use a plain index for it — the same reason migration 7
-- indexes `datetime(created_at)`. And it is partial: only live rows that have
-- a date, which is 7% of this store, because a memory with no review date is
-- not an answer this question ever wants.
--
-- The trailing `id` matches the query's tie-break, so the ordering comes out of
-- the index too and the temporary B-tree disappears with the scan.

CREATE INDEX IF NOT EXISTS idx_obs_review_due
    ON observations(datetime(review_after), id)
    WHERE review_after IS NOT NULL AND deleted_at IS NULL;

-- ---------------------------------------------------------------------------
-- was 0015_review_clocks_a_revision_left_behind.sql
-- ---------------------------------------------------------------------------

-- The review clock, wound for the type a memory ended up being.
--
-- Only three kinds go stale, and `rules::REVIEW_WINDOWS` says how long each
-- stays trustworthy: a decision six months, a policy twelve, a preference
-- three. Everything else is as true in a year as it is today and carries no
-- date at all.
--
-- Revising a memory used to leave that clock alone. A memory saved again under
-- the same topic key goes through the revision path, and its type can change
-- there — a decision that turned out to be a bugfix, a discovery that hardened
-- into a decision — so the store ended up holding both mistakes at once:
-- memories asking to be reread that never go stale, and decisions that will
-- never be asked about. `reschedule_review` fixes it going forward, in both
-- directions, and every row below predates it.
--
-- On a real store of 3,885 live memories:
--
--   19 carried a date their type does not earn — 8 bugfix, 5 architecture,
--      4 discovery, 2 config — every one of them with `revision_count` above 1,
--      and all written between the 23rd and 28th of July.
--   14 were a decision or a preference with no date at all: 12 and 2.
--    0 of the 251 that already had one disagreed with the rule, which is why
--      this only fills the holes rather than rewriting the column.
--
-- Reconstructed from `created_at`, not from now, because the window starts when
-- the memory was written — winding it from today would push a decision from
-- July out to next summer.
--
-- In calendar months, which is what the rule says in words: a decision is good
-- for six months, not for a hundred and eighty days. This file first counted
-- days, because the function it was written from did — one of three places that
-- computed this window, and the only one that disagreed with the other two.
-- They are one function now, `rules::review_after`, and this reads the same way
-- it does. Four days apart on a six-month window, which nobody would have
-- noticed and which is exactly why it was worth removing.
--
-- Nothing falls due the moment this runs: the twelve decisions were written in
-- July and their six months land in January. That was checked before writing
-- this, because a migration that greets somebody with a two-hundred-item review
-- queue is one they will turn off.

UPDATE observations
   SET review_after = NULL
 WHERE review_after IS NOT NULL
   AND type NOT IN ('decision', 'policy', 'preference');

UPDATE observations
   SET review_after = datetime(created_at, '+6 months')
 WHERE review_after IS NULL AND type = 'decision' AND deleted_at IS NULL;

UPDATE observations
   SET review_after = datetime(created_at, '+12 months')
 WHERE review_after IS NULL AND type = 'policy' AND deleted_at IS NULL;

UPDATE observations
   SET review_after = datetime(created_at, '+3 months')
 WHERE review_after IS NULL AND type = 'preference' AND deleted_at IS NULL;

-- ---------------------------------------------------------------------------
-- was 0016_session_activity_index.sql
-- ---------------------------------------------------------------------------

-- The index the session list is grouped through.
--
-- Every opening block asks which sessions were touched most recently, and that
-- means counting a project's memories per session and taking the newest date of
-- each. `idx_obs_session` finds a session's rows and stops there, so the group
-- reads the table for every one of them — bodies included, to use a count and a
-- date.
--
-- Measured on a real store, the same query `recent_sessions` runs, with nothing
-- else changed and no `ANALYZE`:
--
--   project      memories   before     after
--   warden            331   0.11 ms   0.08 ms
--   task-board         542   0.29 ms   0.22 ms
--   almanac            690   2.13 ms   0.16 ms
--   ledgerly         1,712   3.36 ms   0.64 ms
--
-- On ledgerly that was 3.36 ms of a 7.53 ms opening block: half the cost of
-- assembling a session's context, spent on five rows. The plan changes from
-- `SEARCH o USING INDEX idx_obs_session` to `SEARCH o USING COVERING INDEX`,
-- which is the whole of it — the table is not read at all.
--
-- `deleted_at` sits in the middle because the join filters on it, and
-- `created_at` last because it is aggregated rather than searched. The raw
-- column and not `datetime(created_at)`: the query wraps it in `MAX(datetime(…))`
-- and takes a maximum over the group, so what matters here is that the value is
-- in the index at all, not the order it is stored in.
CREATE INDEX IF NOT EXISTS idx_obs_session_activity
ON observations(session_id, deleted_at, created_at);

-- --------------------------------------------------------------------

-- The shelf, without walking the project to find it.
--
-- Every session opening and every `mem_context` asks a project for its pinned
-- memories, and nothing indexed `pinned`. The plan was a `SEARCH` on
-- `idx_obs_project_order`, which sounds fine and is not: that index is
-- `(project, datetime(created_at) DESC, id DESC)`, so the search walks every
-- memory the project has, in date order, testing `pinned = 1` on each. Measured
-- on a store of 41,700 memories, 3,370 of them in that project: 5.57 ms, on the
-- surface that runs before an agent has said anything, and flat in whatever
-- limit the caller asked for.
--
-- Partial, so it holds only what is pinned. On the store this was measured
-- against that is one entry, and the query drops to 0.00 ms. A store with a
-- hundred pins carries a hundred entries, pays nothing for the rest, and only
-- touches the index when somebody pins or unpins.
--
-- On `project` and not on `ifnull(project, '')`, which is the mistake that cost
-- an afternoon: `Narrowing::equals` writes `AND project = ?`, and an index built
-- for the `ifnull` form serves a query nothing issues. Both were built and
-- measured; only this one is used by the statement the code prepares.
CREATE INDEX IF NOT EXISTS idx_obs_pinned
    ON observations(project, datetime(created_at) DESC, id DESC)
 WHERE pinned = 1 AND deleted_at IS NULL;
