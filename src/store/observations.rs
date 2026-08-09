//! Writing, reading and retiring the memories themselves.

use super::*;

/// The page of memories somebody sees when they have not searched for
/// anything, built in one place.
///
/// Named rather than inlined so a test can plan *this* statement. It is the
/// listing behind the dashboard and the CLI, it sorts the whole store, and it
/// is the query that pays for the planner having no statistics: without them
/// SQLite narrows on `deleted_at`, which excludes almost nothing, and sorts
/// what is left in a temporary B-tree.
pub(super) fn unfiltered_page_sql(clause: &str) -> String {
    format!(
        "SELECT {OBSERVATION_COLUMNS} FROM observations
         WHERE deleted_at IS NULL{clause}
         ORDER BY datetime(created_at) DESC, id DESC LIMIT ? OFFSET ?"
    )
}

/// The pinned memories of a project, newest first.
///
/// Named so a guard can explain the statement that runs rather than a copy of
/// it. That distinction is not academic here: `Narrowing::equals` writes
/// `AND project = ?`, an index was built for the `ifnull(project, '')` form
/// nothing issues, and it made no difference to anything because the query it
/// served does not exist.
pub(crate) fn pinned_sql(clauses: &str) -> String {
    format!(
        "SELECT {OBSERVATION_COLUMNS} FROM observations
         WHERE deleted_at IS NULL AND pinned = 1{clauses}
         ORDER BY datetime(created_at) DESC, id DESC"
    )
}

/// Sets, moves or clears a memory's review date to match the type it now has.
///
/// Only three types are ever due for review — `decision`, `policy`,
/// `preference` — and the date used to be written in exactly one place: the
/// insert. Every other way a memory can come to *be* one of those three left it
/// with no date at all, and a memory with no date is one `mem_review` will
/// never name. On a real store, all fourteen decisions and preferences without
/// one had been revised at least once.
///
/// Three ways in, and all three were missing it: `mem_update` changing the
/// type, a save landing on an existing topic key and rewriting it, and a
/// memory arriving over the wire — which the schema does not even carry the
/// column for.
///
/// Not recomputed when the type is unchanged and a date is already set, so
/// fixing a typo does not postpone the review by six months. Cleared when the
/// new type has no window, because a memory that stops being a decision stops
/// being due.
pub(super) fn reschedule_review(
    tx: &Transaction<'_>,
    id: i64,
    kind: &str,
    previous_kind: Option<&str>,
) -> Result<(), StoreError> {
    if crate::memory::rules::review_months(kind).is_none() {
        tx.execute(
            "UPDATE observations SET review_after = NULL WHERE id = ?1",
            [id],
        )?;
        return Ok(());
    }
    let (already, created_at): (Option<String>, String) = tx.query_row(
        "SELECT review_after, created_at FROM observations WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if already.is_some() && previous_kind == Some(kind) {
        return Ok(());
    }
    // Counted from when the memory was written, not from when this store heard
    // about it.
    //
    // The rule in words is "a decision is good for six months", and six months
    // from *what* only ever had one answer: the day it was decided. Migration
    // 15 filled the whole store that way. This counted from `now()`, which is
    // the same thing for a local save — `created_at` is a moment old — and a
    // different thing entirely for a memory arriving over the wire, which is
    // the path this function was written for.
    //
    // A decision made in January and replicated in June came out due in
    // December on the peer and in July on the machine that made it: five months
    // of disagreement about whether it had gone stale. It was found by a guard
    // comparing the two stores, where the two dates differed by one second and
    // would have differed by one second only as long as both ran in the same
    // second.
    //
    // A memory whose type *changes* into a windowed one is dated the same way,
    // and can arrive already due. That is the right answer rather than a
    // side-effect: nobody has confirmed it as a decision in all the time since
    // it was written.
    let from =
        crate::timestamp::parse(&created_at).unwrap_or_else(|| chrono::Utc::now().naive_utc());
    let review = crate::memory::rules::review_after(kind, from);
    tx.execute(
        "UPDATE observations SET review_after = ?1 WHERE id = ?2",
        params![review.map(crate::timestamp::format), id],
    )?;
    Ok(())
}

impl Store {
    pub fn timeline(
        &self,
        observation_id: i64,
        before: Option<usize>,
        after: Option<usize>,
    ) -> Result<TimelineResult, StoreError> {
        // Zero is a question, not a mistake: "just this memory and its session".
        // The schema publishes `minimum: 0` on both — `schemars` derives it from
        // `usize` — and the code raised it to one, so a caller who asked for no
        // neighbours got two. A window around a focus is a section of the
        // answer; asking for none of it is asking for none of it. That is not
        // true of a list's own page size, which is why `review_due` keeps its
        // floor and publishes it.
        // Bounded at the same maximum every other list on this surface has.
        //
        // This one had none, and it is the only reply that can be as large as a
        // session: asking for a window of a million came back with 191 KB — the
        // whole of a 252-memory session — on the surface whose own purpose says
        // a payload that pushes the useful part out of a context window has
        // failed. `before_total` and `after_total` already say how much lies
        // beyond the window, so a bound here costs the caller nothing they are
        // not told about, and the schema publishes it.
        let ceiling = self.config.max_context_results;
        let before = before.unwrap_or(5).min(ceiling);
        let after = after.unwrap_or(5).min(ceiling);
        let focus = get_active_observation(&self.connection, observation_id)?;
        let session_info = get_session_row(&self.connection, &focus.session_id).ok();

        let mut before_statement = self.connection.prepare(
            "SELECT id, session_id, type, title, content, tool_name, project, scope, topic_key,
                    revision_count, duplicate_count, last_seen_at, created_at, updated_at, deleted_at
             FROM observations
             WHERE session_id = ?1 AND id < ?2 AND deleted_at IS NULL
             ORDER BY id DESC LIMIT ?3",
        )?;
        let before_rows = before_statement.query_map(
            params![focus.session_id, observation_id, before as i64],
            map_timeline_entry,
        )?;
        let mut before_entries = before_rows.collect::<Result<Vec<_>, _>>()?;
        before_entries.reverse();

        let mut after_statement = self.connection.prepare(
            "SELECT id, session_id, type, title, content, tool_name, project, scope, topic_key,
                    revision_count, duplicate_count, last_seen_at, created_at, updated_at, deleted_at
             FROM observations
             WHERE session_id = ?1 AND id > ?2 AND deleted_at IS NULL
             ORDER BY id ASC LIMIT ?3",
        )?;
        let after_rows = after_statement.query_map(
            params![focus.session_id, observation_id, after as i64],
            map_timeline_entry,
        )?;
        let after_entries = after_rows.collect::<Result<Vec<_>, _>>()?;
        // How much of the session is on each side, rather than how big the
        // session is.
        //
        // This used to be one number called `total_in_range` holding the whole
        // session's count — 221 on a real store, for every focus, whatever
        // window was asked for. A caller comparing it against the lists beside
        // it read "221 in range" over seven entries, which is the same defect
        // `ReviewOutput::count` had: a field answering a different question
        // from the one its name asks.
        //
        // Two counts say what one could not. `before` and `after` are capped by
        // the window, so a full list and an exhausted one look alike, and which
        // side has more is what decides whether to ask again — a focus can be
        // the first memory of a long session or the last. Both are index range
        // scans over `(session_id, id)`, and the session total is still there
        // for anyone who wants it: it is these two and the focus.
        let side_total = |comparison: &str| -> Result<i64, StoreError> {
            Ok(self.connection.query_row(
                &format!(
                    "SELECT COUNT(*) FROM observations
                     WHERE session_id = ?1 AND id {comparison} ?2 AND deleted_at IS NULL"
                ),
                params![focus.session_id, observation_id],
                |row| row.get(0),
            )?)
        };
        let before_total = side_total("<")?;
        let after_total = side_total(">")?;

        Ok(TimelineResult {
            focus,
            before: before_entries,
            after: after_entries,
            session_info,
            before_total,
            after_total,
        })
    }

    pub fn add_observation(&mut self, input: AddObservation) -> Result<AddOutcome, StoreError> {
        let (kind, title, content, project, scope, topic_key, hash) = normalize::fields(
            &input.kind,
            &input.title,
            &input.content,
            input.project.as_deref(),
            &input.scope,
            input.topic_key.as_deref(),
            self.config.max_observation_length,
        )
        .into_parts();
        // The door. Rejection is a rule, so it lives in `rules` and every entry
        // point gets the same answer — the empty-content check used to exist
        // only in the MCP adapter, which meant the CLI could write a memory
        // that recorded that something happened and not what.
        if let Some(refusal) = crate::memory::rules::refuse(&title, &content) {
            return Err(invalid_parameter(refusal.message()));
        }
        // An empty string would record a link to a prompt that does not exist.
        let prompt_sync_id = input
            .prompt_sync_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let dedupe_minutes = self.config.dedupe_window.as_secs().div_ceil(60) as i64;

        let tx = self.write_transaction()?;
        ensure_session_tx(&tx, &input.session_id)?;
        if let Some(topic_key) = &topic_key {
            let existing = tx
                .query_row(
                    "SELECT id FROM observations
                     WHERE topic_key = ?1 AND ifnull(project, '') = ifnull(?2, '')
                       AND scope = ?3 AND deleted_at IS NULL
                     ORDER BY datetime(updated_at) DESC, datetime(created_at) DESC LIMIT 1",
                    params![topic_key, project, scope],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if let Some(id) = existing {
                let previous_kind: String =
                    tx.query_row("SELECT type FROM observations WHERE id = ?1", [id], |row| {
                        row.get(0)
                    })?;
                tx.execute(
                    "UPDATE observations SET type = ?1, title = ?2, content = ?3, tool_name = ?4,
                     topic_key = ?5, normalized_hash = ?6, revision_count = revision_count + 1,
                     last_seen_at = datetime('now'), updated_at = datetime('now') WHERE id = ?7",
                    params![kind, title, content, input.tool_name, topic_key, hash, id],
                )?;
                reschedule_review(&tx, id, &kind, Some(&previous_kind))?;
                let observation = get_observation_row(&tx, id)?;
                enqueue_observation(&tx, &observation)?;
                tx.commit()?;
                return Ok(AddOutcome {
                    kind: AddOutcomeKind::Revised,
                    observation,
                });
            }
        }

        let modifier = normalize::sqlite_datetime_modifier(dedupe_minutes);
        let existing = tx
            .query_row(
                "SELECT id FROM observations
                 WHERE normalized_hash = ?1 AND ifnull(project, '') = ifnull(?2, '')
                   AND scope = ?3 AND type = ?4 AND title = ?5 AND deleted_at IS NULL
                   AND datetime(created_at) >= datetime('now', ?6)
                 ORDER BY created_at DESC LIMIT 1",
                params![hash, project, scope, kind, title, modifier.as_ref()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        if let Some(id) = existing {
            tx.execute(
                "UPDATE observations SET duplicate_count = duplicate_count + 1,
                 last_seen_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1",
                [id],
            )?;
            let observation = get_observation_row(&tx, id)?;
            enqueue_observation(&tx, &observation)?;
            tx.commit()?;
            return Ok(AddOutcome {
                kind: AddOutcomeKind::Deduplicated,
                observation,
            });
        }

        let sync_id = normalize::sync_id("obs");
        tx.execute(
            "INSERT INTO observations
             (sync_id, session_id, type, title, content, tool_name, project, scope, topic_key,
              normalized_hash, prompt_sync_id, revision_count, duplicate_count, last_seen_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, 1, datetime('now'), datetime('now'))",
            params![sync_id, input.session_id, kind, title, content, input.tool_name, project, scope, topic_key, hash, prompt_sync_id],
        )?;
        let id = tx.last_insert_rowid();
        // From the row's own `created_at`, through the one function that knows
        // this rule, rather than from a second reading of the clock.
        //
        // It was `Utc::now()` here and `created_at` on the wire, and the note in
        // `reschedule_review` says why that looked safe: "the same thing for a
        // local save — `created_at` is a moment old". A moment is not nothing.
        // `created_at` comes from SQLite's `datetime('now')` inside the INSERT
        // and this ran a few microseconds later in Rust, so a save that crossed
        // a second boundary between the two got a review date one second past
        // the one every other machine would compute from the same memory — and
        // the replication guard, which compares the two stores field by field,
        // failed on it about twice in twenty-five runs of the suite. Rare
        // because the window is microseconds wide, and commoner under load
        // because that is what widens it.
        //
        // Two clocks for one rule, which is the shape this crate keeps finding.
        // Now there is one, and it reads the value both sides already agree on.
        reschedule_review(&tx, id, &kind, None)?;
        let observation = get_observation_row(&tx, id)?;
        enqueue_observation(&tx, &observation)?;
        tx.commit()?;
        Ok(AddOutcome {
            kind: AddOutcomeKind::Inserted,
            observation,
        })
    }

    pub fn get_observation(&self, id: i64) -> Result<Observation, StoreError> {
        get_observation_row(&self.connection, id)
    }

    pub fn update_observation(
        &mut self,
        id: i64,
        input: UpdateObservation,
    ) -> Result<Observation, StoreError> {
        let max_length = self.config.max_observation_length;
        let tx = self.write_transaction()?;
        let current = get_active_observation(&tx, id)?;

        // Every field normalises what the caller supplied and leaves what it
        // did not. `kind` was the exception: an update could write back the
        // `bug` that a save folds to `bugfix`, so the same word meant two
        // things depending on which call wrote it.
        let previous_kind = current.kind.clone();
        let kind = input
            .kind
            .map(|value| normalize::kind(&value))
            .unwrap_or_else(|| previous_kind.clone());
        let title = input
            .title
            .map(|value| normalize::title(&value, max_length))
            .unwrap_or(current.title);
        let content = input
            .content
            .map(|value| normalize::truncate_content(normalize::strip_private(&value), max_length))
            .unwrap_or(current.content);
        // The same door as saving. Closing it on the write path alone left the
        // back way open: an update could blank a title that was already there,
        // which is worse than never having one.
        if let Some(refusal) = crate::memory::rules::refuse(&title, &content) {
            return Err(invalid_parameter(refusal.message()));
        }
        // Kept because the wire needs it: a memory can leave a project that is
        // being replicated, and what the peer has to be told is about the
        // project it is watching rather than about the one this row landed in.
        let previous_project = current.project.clone();
        let project = input
            .project
            .map(|value| normalize::project(&value))
            .map(|value| (!value.is_empty()).then_some(value))
            .unwrap_or(current.project);
        let scope = input
            .scope
            .map(|value| normalize::scope(&value).to_owned())
            .unwrap_or(current.scope);
        let topic_key = input
            .topic_key
            .map(|value| normalize::topic_key(Some(&value)))
            .unwrap_or(current.topic_key);
        let hash = normalize::normalized_hash(&content);

        let changed = tx.execute(
            "UPDATE observations
             SET type = ?1, title = ?2, content = ?3, project = ?4, scope = ?5,
                 topic_key = ?6, normalized_hash = ?7, revision_count = revision_count + 1,
                 updated_at = datetime('now')
             WHERE id = ?8 AND deleted_at IS NULL",
            params![kind, title, content, project, scope, topic_key, hash, id],
        )?;
        if changed == 0 {
            // Which of the two it was, the way every other door answers it: an
            // `UPDATE` that changed nothing cannot tell an absent row from a
            // tombstoned one, and saying "not found" about a memory sitting in
            // the table sends whoever asked to doubt their own id.
            return Err(deleted_or_missing(&tx, id));
        }
        reschedule_review(&tx, id, &kind, Some(previous_kind.as_str()))?;
        let observation = get_active_observation(&tx, id)?;
        // A memory that changes project also leaves proposals behind, and they
        // are as stranded as the ghost the block below is about: a relation
        // joins two memories of one project, so anything still pending against
        // the project this memory just left can never be judged again. Marked
        // here rather than filtered by every reader, because a pending row that
        // no call can ever settle is counted in every queue that counts pending
        // rows, and a queue that cannot reach zero is one people learn to skip.
        strand_relations_tx(
            &tx,
            &observation.sync_id,
            observation.project.as_deref().unwrap_or_default(),
        )?;
        enqueue_observation(&tx, &observation)?;
        // A memory that walked out of a replicated project leaves a ghost
        // behind unless somebody says so.
        //
        // The queue writes under the project a row is in *now*, and drops
        // anything whose project nobody replicates — so moving a memory from an
        // enrolled project to an unenrolled one queued nothing at all, and the
        // peer went on holding it under the old name, with the old body,
        // for ever. Nothing said so, which is the same silence
        // `merge_projects` was fixed for: there the canonical project takes
        // over the source's enrolment, because the memories are the same set
        // under a new name. Here they are not — enrolling the destination would
        // start replicating a project nobody asked to replicate — so what
        // travels is the only thing that is true from where the peer is
        // standing: it is gone from the project you are watching.
        //
        // Only in that direction. Into an enrolled project the upsert above
        // already carries it, and between two enrolled projects the row itself
        // names its new project, so the peer follows it.
        let left = previous_project.as_deref().unwrap_or_default();
        let arrived = observation.project.as_deref().unwrap_or_default();
        if left != arrived && is_enrolled_tx(&tx, left)? && !is_enrolled_tx(&tx, arrived)? {
            let payload = serde_json::json!({
                "sync_id": observation.sync_id,
                "session_id": observation.session_id,
                "project": left,
                "deleted": true,
                "deleted_at": crate::timestamp::now(),
                "hard_delete": false,
            });
            enqueue_mutation(
                &tx,
                "observation",
                &observation.sync_id,
                crate::sync::OP_DELETE,
                &payload,
                left,
            )?;
        }
        tx.commit()?;
        Ok(observation)
    }

    /// How many live memories the store holds outside one project, up to `cap`.
    ///
    /// Asked only when a project-narrowed read came back with nothing, to tell
    /// the difference between a store that is empty and a directory that
    /// resolved somewhere quiet.
    ///
    /// Bounded, and that is the whole design. `project <> ?` is not a range, so
    /// no index answers it and an exact count reads every live row — 8 ms on a
    /// store of 3,948, growing with the store, on the path a session opens
    /// with. Worse, it made the empty answer the expensive one: the same hook
    /// cost 16.6 ms where there was work to do and 23.9 ms where there was
    /// none.
    ///
    /// Stopping at `cap` makes it constant in the size of the store, and costs
    /// nothing that matters: the sentence exists to answer "am I in the wrong
    /// project", which any number at all answers. The caller says so when the
    /// count stopped early — see `no_match_here_hint`.
    pub fn memories_outside(&self, project: &str, cap: usize) -> Result<i64, StoreError> {
        let project = normalize::project(project);
        let summary = crate::memory::model::SESSION_SUMMARY;
        Ok(self.connection.query_row(
            &format!(
                "SELECT COUNT(*) FROM (
                     SELECT 1 FROM observations
                      WHERE deleted_at IS NULL AND type <> '{summary}'
                        AND ifnull(project, '') <> ?1
                      LIMIT ?2
                 )"
            ),
            params![project, cap as i64],
            |row| row.get(0),
        )?)
    }

    /// The memories an opening block lists, and only those.
    ///
    /// [`recent_observations`](Self::recent_observations) answers with
    /// everything and leaves the caller to drop what it cannot use, which meant
    /// asking for four times the budget and hoping: pinned memories are listed
    /// separately, session summaries are folded onto their sessions, and a
    /// narrowed scope is filtered afterwards. Four times over is a guess, and
    /// what a guess costs is either too much read or too little delivered —
    /// on a real store, 360KB of memory bodies fetched to show 175KB of them.
    ///
    /// Saying it in SQL asks for exactly what will be shown. The three
    /// conditions are the same three the caller applied by hand; the difference
    /// is that SQLite counts the `LIMIT` after them rather than before.
    pub fn recent_memories(
        &self,
        project: Option<&str>,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Observation>, StoreError> {
        let project = project.map(normalize::project);
        // Blank is absent — see `Store::search`, where the same fold narrowed
        // an answer to project scope without saying so.
        let scope = normalize::optional(scope).as_deref().map(normalize::scope);
        let limit = limit.max(1) as i64;
        let summary = crate::memory::model::SESSION_SUMMARY;
        let mut narrowing = Narrowing::new();
        narrowing.equals("project", project.as_ref());
        narrowing.equals("scope", scope.as_ref());
        let limit = narrowing.bind(&limit);
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM observations
             WHERE deleted_at IS NULL AND pinned = 0 AND type <> '{summary}'{}
             ORDER BY datetime(created_at) DESC, id DESC LIMIT ?{limit}",
            narrowing.clauses()
        ))?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(narrowing.values()),
            map_observation,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The summary each of these sessions wrote, looked up by session.
    ///
    /// The fold used to take whatever summaries happened to fall inside the
    /// recent-memory window, which is not the same question. A session whose
    /// summary is older than that window — anything saved since pushed it out —
    /// appeared in the opening block as a name and a date with nothing about
    /// what it was for, and the summary was not listed as a memory either,
    /// because the fold had already set it aside. On a real store that silently
    /// emptied 3 of the 19 recent sessions that had one to show.
    ///
    /// One row per session, which is what the fold uses and was not what this
    /// returned.
    ///
    /// "A session has at most one summary" is what the note here used to say,
    /// and clients disagree: an agent that reuses a session id writes one every
    /// time it finishes something, so a real store holds 71 summaries under
    /// `improve-engine-20260607-1852`, 39 under `codex-54400d2b` and 37 under
    /// `codex-current` — 101 session ids with more than one, and every summary
    /// genuinely different text rather than the same one saved twice.
    ///
    /// The fold takes the newest of them and drops the rest, so the rest were
    /// read for nothing — with their bodies, which is what a summary mostly is.
    /// On the same store, the five most recent sessions of one project brought
    /// back 19 summaries and 58.8 KB to render two lines out of 6.3 KB of it.
    /// That runs at every session opening and on every `mem_context`.
    ///
    /// So the newest per session is chosen in SQL. The fold still looks for the
    /// first row matching each session and is unchanged; there is simply one
    /// left for it to find.
    pub fn session_summaries(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<Observation>, StoreError> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let summary = crate::memory::model::SESSION_SUMMARY;
        let holes = std::iter::repeat_n("?", session_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM (
                 SELECT *, ROW_NUMBER() OVER (
                            PARTITION BY session_id
                            ORDER BY datetime(created_at) DESC, id DESC
                        ) AS place
                   FROM observations
                  WHERE deleted_at IS NULL AND type = '{summary}'
                    AND session_id IN ({holes})
             )
             WHERE place = 1
             ORDER BY datetime(created_at) DESC, id DESC"
        ))?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(session_ids.iter()),
            map_observation,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The most recent memories of a project, newest first.
    ///
    /// `summaries` says whether the session summaries count. Every other
    /// surface that answers "what happened recently" leaves them out — the
    /// opening block, `mem_context`, the memories a prompt hint may name, the
    /// widened stages of a search — because a summary is about a session rather
    /// than a thing somebody learned, and the sessions are listed on their own
    /// beside it. This one included them, so `leteo recent --limit 20` came
    /// back with seven of twenty in one real project and eight in another: a
    /// third of the answer to "what have I been doing" spent on the covers of
    /// the book.
    ///
    /// Said at each door rather than decided here, because the callers do not
    /// agree and each has a reason. The save reminder counts a summary as
    /// something kept, which it is. The Obsidian export writes them out like
    /// any other memory. The conflict scan does not: a summary touches
    /// everything, which is why `find_candidates` already refuses to propose
    /// one, and proposing *from* one is the same shape.
    pub fn recent_observations(
        &self,
        project: Option<&str>,
        limit: Option<usize>,
        summaries: bool,
    ) -> Result<Vec<Observation>, StoreError> {
        let project = project.map(normalize::project);
        let limit = limit.unwrap_or(self.config.max_context_results).max(1) as i64;
        let mut narrowing = Narrowing::new();
        narrowing.equals("project", project.as_ref());
        let limit = narrowing.bind(&limit);
        let summary = crate::memory::model::SESSION_SUMMARY;
        let without = if summaries {
            String::new()
        } else {
            format!(" AND type <> '{summary}'")
        };
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM observations
             WHERE deleted_at IS NULL{without}{}
             ORDER BY datetime(created_at) DESC, id DESC LIMIT ?{limit}",
            narrowing.clauses()
        ))?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(narrowing.values()),
            map_observation,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// How many live memories a project holds, or the whole store when no
    /// project is named.
    ///
    /// Counted in SQLite rather than by asking for the rows and measuring them:
    /// the callers want the number for a single sentence, and a busy project
    /// answers that question with several hundred rows it would then drop.
    pub fn count_observations(&self, project: Option<&str>) -> Result<i64, StoreError> {
        let project = project.map(normalize::project);
        let mut narrowing = Narrowing::new();
        narrowing.equals("project", project.as_ref());
        let sql = format!(
            "SELECT COUNT(*) FROM observations WHERE deleted_at IS NULL{}",
            narrowing.clauses()
        );
        self.connection
            .query_row(
                &sql,
                rusqlite::params_from_iter(narrowing.values()),
                |row| row.get(0),
            )
            .map_err(StoreError::from)
    }

    /// One page of observations, narrowed by project and by words.
    ///
    /// The two narrowings compose, and either may be absent: an empty project
    /// set is every project, and an empty query is no search — which is what
    /// puts the recent rows back rather than being a mistake, because clearing
    /// the search box is how somebody stops searching.
    ///
    /// With a query the rows come back best-match first; without one, newest
    /// first. That is the same list under two orders rather than two lists, so
    /// the screen showing it needs one cursor and one way to open a row.
    ///
    /// The caller's limit is honoured rather than clamped to
    /// `max_search_results`. That cap keeps an agent's tool reply small, and
    /// this is a screen somebody scrolls: capped at twenty, a page of matches
    /// once sat beside a session claiming twenty-three of them, and the screen
    /// contradicted itself.
    pub fn paged_observations(
        &self,
        query: &str,
        projects: &[String],
        offset: usize,
        limit: usize,
    ) -> Result<Listing<Observation>, StoreError> {
        let limit = limit.max(1) as i64;
        let offset = offset as i64;
        // Prepared first, and the branch is on what came out of it: a query of
        // nothing but punctuation leaves no terms at all, and `MATCH ''` is a
        // syntax error rather than a search that finds nothing.
        let fts = normalize::fts_prefix_query(query);
        if fts.is_empty() {
            let (clause, values) = Self::project_clause(projects, "project");
            let bound: Vec<&dyn rusqlite::ToSql> =
                values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
            let total = self.connection.query_row(
                &format!("SELECT COUNT(*) FROM observations WHERE deleted_at IS NULL{clause}"),
                bound.as_slice(),
                |row| row.get(0),
            )?;
            let mut statement = self.connection.prepare(&unfiltered_page_sql(&clause))?;
            let mut bound = bound;
            bound.push(&limit);
            bound.push(&offset);
            let rows = statement
                .query_map(bound.as_slice(), map_observation)?
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Listing { rows, total });
        }

        let (clause, values) = Self::project_clause(projects, "o.project");
        let mut bound: Vec<&dyn rusqlite::ToSql> = vec![&fts];
        bound.extend(values.iter().map(|v| v as &dyn rusqlite::ToSql));
        let total = self.connection.query_row(
            &format!(
                "SELECT COUNT(*)
                 FROM observations_fts fts CROSS JOIN observations o ON o.id = fts.rowid
                 WHERE observations_fts MATCH ? AND o.deleted_at IS NULL{clause}"
            ),
            bound.as_slice(),
            |row| row.get(0),
        )?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS_JOINED}
             FROM observations_fts fts CROSS JOIN observations o ON o.id = fts.rowid
             WHERE observations_fts MATCH ? AND o.deleted_at IS NULL{clause}
             ORDER BY bm25(observations_fts, {BM25_WEIGHTS})
             LIMIT ? OFFSET ?"
        ))?;
        bound.push(&limit);
        bound.push(&offset);
        let rows = statement
            .query_map(bound.as_slice(), map_observation)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Listing { rows, total })
    }

    /// One page of what a session recorded, oldest first.
    ///
    /// Oldest first because a session is a sequence: read top to bottom it is
    /// the order the work happened in, which is what somebody opening a session
    /// came to see. Every other list here is newest first, because those are
    /// asking "what is going on now" instead.
    pub fn paged_session_observations(
        &self,
        session_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Listing<Observation>, StoreError> {
        let limit = limit.max(1) as i64;
        let offset = offset as i64;
        let total = self.connection.query_row(
            "SELECT COUNT(*) FROM observations WHERE session_id = ?1 AND deleted_at IS NULL",
            params![session_id],
            |row| row.get(0),
        )?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM observations
             WHERE session_id = ?1 AND deleted_at IS NULL
             ORDER BY datetime(created_at) ASC, id ASC LIMIT ?2 OFFSET ?3"
        ))?;
        let rows = statement
            .query_map(params![session_id, limit, offset], map_observation)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Listing { rows, total })
    }

    /// The memories somebody put on the shelf, newest first, and how many did
    /// not fit.
    ///
    /// Bounded, which it was not. Pins are listed on top of a context's budget
    /// rather than inside it — a project with as many pins as the budget got
    /// its pins and nothing else, and the reward for deciding what matters must
    /// not be to stop being told what happened — but on top of a bound is not
    /// the same as outside every bound. With 360 pinned memories, `mem_context`
    /// answered 370 of them in 229.5 KB with a ceiling of 80 in force on the
    /// other list, and the opening block, which nobody can pass a limit to,
    /// carried the same 370 into every session start.
    ///
    /// So each list has its own ceiling and neither starves the other. The
    /// count of what was left out is returned rather than swallowed: a pin is
    /// the most deliberate thing in the store, and dropping one silently is
    /// worse than the bytes it would have cost.
    pub fn pinned_observations(
        &self,
        project: Option<&str>,
        scope: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Observation>, usize), StoreError> {
        let project = project.map(normalize::project);
        // Blank is absent, as above.
        let scope = normalize::optional(scope).as_deref().map(normalize::scope);
        let mut narrowing = Narrowing::new();
        narrowing.equals("project", project.as_ref());
        narrowing.equals("scope", scope.as_ref());
        let mut statement = self.connection.prepare(&pinned_sql(narrowing.clauses()))?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(narrowing.values()),
            map_observation,
        )?;
        // Read whole and cut here rather than with a `LIMIT`, because the
        // number left behind is the half worth reporting and SQL would have
        // thrown it away. The rows are titles and previews of a shelf somebody
        // curated by hand; the query is the same one that was already running.
        let mut rows = rows.collect::<Result<Vec<_>, _>>()?;
        let omitted = rows.len().saturating_sub(limit);
        rows.truncate(limit);
        Ok((rows, omitted))
    }

    pub fn pin_observation(&mut self, id: i64) -> Result<(), StoreError> {
        self.set_observation_pinned(id, true)
    }

    pub fn unpin_observation(&mut self, id: i64) -> Result<(), StoreError> {
        self.set_observation_pinned(id, false)
    }

    fn set_observation_pinned(&mut self, id: i64, pinned: bool) -> Result<(), StoreError> {
        let tx = self.write_transaction()?;
        let changed = tx.execute(
            "UPDATE observations SET pinned = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            params![pinned, id],
        )?;
        if changed == 0 {
            return Err(deleted_or_missing(&tx, id));
        }
        tx.commit()?;
        Ok(())
    }

    /// What makes a memory due for a reread, in one place.
    ///
    /// The list and the count are the same question asked twice — a session
    /// opening says how many there are and `mem_review` hands them over — and a
    /// second copy of this clause is how one of them would come to disagree
    /// with the other about what "due" means.
    const REVIEW_DUE: &'static str = "deleted_at IS NULL AND review_after IS NOT NULL
               AND datetime(review_after) <= datetime('now')";

    /// How many memories are due, without reading any of them.
    ///
    /// Four microseconds on a real store: `idx_obs_review_due` is a partial
    /// index on `datetime(review_after)` that migration 14 added for exactly
    /// this shape, and the count never leaves it.
    pub fn count_review_due(&self, project: Option<&str>) -> Result<i64, StoreError> {
        let project = project.map(normalize::project);
        let mut narrowing = Narrowing::new();
        narrowing.equals("project", project.as_ref());
        let sql = format!(
            "SELECT COUNT(*) FROM observations WHERE {}{}",
            Self::REVIEW_DUE,
            narrowing.clauses()
        );
        self.connection
            .query_row(
                &sql,
                rusqlite::params_from_iter(narrowing.values()),
                |row| row.get(0),
            )
            .map_err(StoreError::from)
    }

    pub fn review_due(
        &self,
        project: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Observation>, StoreError> {
        let project = project.map(normalize::project);
        let limit = limit.unwrap_or(self.config.max_context_results).max(1) as i64;
        let mut narrowing = Narrowing::new();
        narrowing.equals("project", project.as_ref());
        let limit = narrowing.bind(&limit);
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM observations
             WHERE {}{}
             ORDER BY datetime(review_after) ASC, id ASC LIMIT ?{limit}",
            Self::REVIEW_DUE,
            narrowing.clauses()
        ))?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(narrowing.values()),
            map_observation,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn observations_needing_review(
        &self,
        project: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Observation>, StoreError> {
        self.review_due(project, limit)
    }

    pub fn mark_reviewed(&mut self, id: i64) -> Result<(), StoreError> {
        let tx = self.write_transaction()?;
        let observation = get_active_observation(&tx, id)?;
        let review_after =
            crate::memory::rules::review_after(&observation.kind, Utc::now().naive_utc())
                .map(crate::timestamp::format);
        let changed = tx.execute(
            "UPDATE observations SET review_after = ?1, updated_at = datetime('now')
             WHERE id = ?2 AND deleted_at IS NULL",
            params![review_after, id],
        )?;
        if changed == 0 {
            return Err(deleted_or_missing(&tx, id));
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_observation(&mut self, id: i64, hard_delete: bool) -> Result<(), StoreError> {
        let tx = self.write_transaction()?;
        let observation = if hard_delete {
            get_observation_row(&tx, id)?
        } else {
            get_active_observation(&tx, id)?
        };
        let deleted_at = sqlite_now();
        if hard_delete {
            tx.execute("DELETE FROM observations WHERE id = ?1", [id])?;
            orphan_relations_tx(&tx, &observation.sync_id)?;
        } else {
            let changed = tx.execute(
                "UPDATE observations SET deleted_at = ?1, updated_at = datetime('now')
                 WHERE id = ?2 AND deleted_at IS NULL",
                params![deleted_at, id],
            )?;
            if changed == 0 {
                return Err(deleted_or_missing(&tx, id));
            }
        }
        let project = observation.project.as_deref().unwrap_or_default();
        let payload = serde_json::json!({
            "sync_id": observation.sync_id,
            "session_id": observation.session_id,
            "project": observation.project,
            "deleted": true,
            "deleted_at": deleted_at,
            "hard_delete": hard_delete,
        });
        enqueue_mutation(
            &tx,
            "observation",
            &observation.sync_id,
            crate::sync::OP_DELETE,
            &payload,
            project,
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn passive_capture(
        &mut self,
        input: PassiveCapture,
    ) -> Result<PassiveCaptureResult, StoreError> {
        let project = normalize::project(&input.project);
        let learnings = normalize::extract_learnings(&input.content);
        // Bounded here rather than in the extractor, because the extractor is
        // about what a text says and this is about what one turn may leave
        // behind — and because the number that did not fit is worth reporting,
        // which a truncated list cannot say. See `normalize::MAX_LEARNINGS`.
        let dropped = learnings.len().saturating_sub(normalize::MAX_LEARNINGS);
        let mut result = PassiveCaptureResult {
            extracted: learnings.len(),
            dropped,
            ..PassiveCaptureResult::default()
        };
        for learning in learnings.into_iter().take(normalize::MAX_LEARNINGS) {
            // Hashed the way the store hashes what it keeps, not the way it
            // arrived. This check is the one with no time window under it — the
            // reason a subagent stopping tomorrow does not file the same
            // learning again — so a hash that cannot match is a store that
            // collects copies.
            let (_, hash) =
                normalize::stored_content(&learning, self.config.max_observation_length);
            let duplicate = self.connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM observations WHERE normalized_hash = ?1
                      AND ifnull(project, '') = ?2 AND deleted_at IS NULL
                 )",
                params![hash, project],
                |row| row.get::<_, bool>(0),
            )?;
            if duplicate {
                result.duplicates += 1;
                continue;
            }
            // Cut between words, because this is a row rather than a
            // rendering: see `normalize::truncate_words`. At the same bound
            // every surface that shows a title uses, because a title cut
            // shorter than that is cut once here and never shown cut at all —
            // see `normalize::TITLE_CHARS`.
            let title = normalize::truncate_words(&learning, normalize::TITLE_CHARS);
            let outcome = self.add_observation(AddObservation {
                session_id: input.session_id.clone(),
                kind: "passive".to_owned(),
                title,
                content: learning,
                tool_name: normalize::optional(Some(&input.source)),
                project: (!project.is_empty()).then_some(project.clone()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })?;
            // What was stored, rather than what was handed over. There is a
            // second, narrower guard inside `add_observation`, and counting the
            // call as a save meant a learning it folded into an existing row
            // was still announced as captured — a number nobody could check
            // against the store it claims to describe.
            match outcome.kind {
                AddOutcomeKind::Inserted => result.saved += 1,
                AddOutcomeKind::Revised | AddOutcomeKind::Deduplicated => result.duplicates += 1,
            }
        }
        Ok(result)
    }
}
