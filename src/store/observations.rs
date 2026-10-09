//! Writing, reading and retiring the memories themselves.

use super::*;

/// Why a passive capture stopped, and what it managed to store first.
///
/// Carried rather than folded into [`StoreError`] because the count is a fact
/// about the run and not about the failure: the caller that says what happened
/// needs both, and the one that only reports an error can take the error alone.
#[derive(Debug)]
pub struct CaptureFailure {
    pub error: StoreError,
    /// What reached the store before the error stopped the capture.
    pub partial: PassiveCaptureResult,
}

/// The page of memories somebody sees when they have not searched for
/// anything, built in one place.
///
/// Named rather than inlined so a test can plan *this* statement. It is the
/// listing behind the dashboard and the CLI, it sorts the whole store, and it
/// is the query that pays for the planner having no statistics: without them
/// SQLite narrows on `deleted_at`, which excludes almost nothing, and sorts
/// what is left in a temporary B-tree.
pub(super) fn unfiltered_page_sql(clause: &str) -> String {
    let not_superseded = super::relations::not_superseded();
    format!(
        "SELECT {OBSERVATION_COLUMNS} FROM observations o
         WHERE o.deleted_at IS NULL AND {not_superseded}{clause}
         ORDER BY datetime(o.created_at) DESC, o.id DESC LIMIT ? OFFSET ?"
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
    let not_superseded = super::relations::not_superseded();
    format!(
        "SELECT {OBSERVATION_COLUMNS} FROM observations o
         WHERE o.deleted_at IS NULL AND o.pinned = 1 AND {not_superseded}{clauses}
         ORDER BY datetime(o.created_at) DESC, o.id DESC"
    )
}

/// Refuses a write whose caller asserted a project the memory is not in.
///
/// The four mutating tools take an `expected_project`; this is the one place
/// that compares it to the row's stored project. It runs inside the same write
/// transaction as the change it guards, so a refusal leaves the row, its
/// revision count and the sync queue exactly as they were. `None` means the
/// caller makes no assertion — the CLI and the TUI act on an id a person chose,
/// not on an id an agent copied out of a cross-project search.
fn assert_expected_project(
    id: i64,
    actual: Option<&str>,
    expected: Option<&str>,
) -> Result<(), StoreError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let expected = normalize::project(expected);
    let actual = normalize::project(actual.unwrap_or_default());
    if expected == actual {
        return Ok(());
    }
    Err(StoreError::ProjectMismatch {
        id,
        expected: project_label(&expected),
        actual: project_label(&actual),
    })
}

fn project_label(project: &str) -> String {
    if project.is_empty() {
        "no project".to_owned()
    } else {
        format!("{project:?}")
    }
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

/// Writes one new memory, inside a caller's transaction.
///
/// The INSERT lived in `add_observation` alone until a second path needed to
/// write a memory — `consolidate_observations`, which must insert the
/// replacement in the same transaction as the relations that point at it. The
/// column list, the review clock and the replication journal are one rule, so
/// they are one function rather than two copies one edit apart.
///
/// The caller has already normalised through [`normalize::fields`] and refused
/// what the store will not hold; this is the write and nothing else.
fn insert_observation_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    tool_name: Option<&str>,
    prompt_sync_id: Option<&str>,
    fields: normalize::Fields,
) -> Result<Observation, StoreError> {
    let (kind, title, content, project, scope, topic_key, hash) = fields.into_parts();
    let sync_id = normalize::sync_id("obs");
    tx.execute(
        "INSERT INTO observations
         (sync_id, session_id, type, title, content, tool_name, project, scope, topic_key,
          normalized_hash, prompt_sync_id, revision_count, duplicate_count, last_seen_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, 1, datetime('now'), datetime('now'))",
        params![sync_id, session_id, kind, title, content, tool_name, project, scope, topic_key, hash, prompt_sync_id],
    )?;
    let id = tx.last_insert_rowid();
    reschedule_review(tx, id, &kind, None)?;
    let observation = get_observation_row(tx, id)?;
    enqueue_observation(tx, &observation)?;
    Ok(observation)
}

/// Records the title and body a content-changing write replaced, and keeps only
/// the newest [`OBSERVATION_VERSION_RETENTION`] of them.
///
/// One implementation for both write paths. The local door calls it with the
/// row it is about to overwrite; the replicated door calls it with the bytes a
/// peer's payload already carries, so the two machines hold the same version
/// rather than each snapshotting its own idea of the previous text.
///
/// `INSERT OR IGNORE` against the `(observation_sync_id, revision)` unique index
/// is what makes applying a payload twice harmless: the second attempt is a
/// no-op rather than a duplicate row. The return is the rows that insert
/// actually added, which is how an import counts what it restored; the retention
/// delete that follows is not part of it.
pub(super) fn snapshot_observation_version_tx(
    tx: &Transaction<'_>,
    observation_sync_id: &str,
    revision: i64,
    title: &str,
    content: &str,
    replaced_at: &str,
) -> Result<usize, StoreError> {
    if observation_sync_id.is_empty() {
        return Ok(0);
    }
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO observation_versions
         (observation_sync_id, revision, title, content, replaced_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![observation_sync_id, revision, title, content, replaced_at],
    )?;
    // The oldest row past the bound names the cut: `revision` only grows for
    // one observation, so everything at or below it is older than the newest N.
    // The subquery answers NULL while the observation holds fewer than N
    // versions, and `<= NULL` deletes nothing.
    tx.execute(
        "DELETE FROM observation_versions
          WHERE observation_sync_id = ?1
            AND revision <= (
                SELECT revision FROM observation_versions
                 WHERE observation_sync_id = ?1
                 ORDER BY revision DESC
                 LIMIT 1 OFFSET ?2
            )",
        params![observation_sync_id, OBSERVATION_VERSION_RETENTION as i64],
    )?;
    Ok(inserted)
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
        let fields = normalize::fields(
            &input.kind,
            &input.title,
            &input.content,
            input.project.as_deref(),
            &input.scope,
            input.topic_key.as_deref(),
            self.config.max_observation_length,
        );
        // The door. Rejection is a rule, so it lives in `rules` and every entry
        // point gets the same answer — the empty-content check used to exist
        // only in the MCP adapter, which meant the CLI could write a memory
        // that recorded that something happened and not what.
        if let Some(refusal) = crate::memory::rules::refuse(fields.title(), fields.content()) {
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
        if let Some(topic_key) = fields.topic_key() {
            let existing = tx
                .query_row(
                    "SELECT id FROM observations
                     WHERE topic_key = ?1 AND ifnull(project, '') = ifnull(?2, '')
                       AND scope = ?3 AND deleted_at IS NULL
                     ORDER BY datetime(updated_at) DESC, datetime(created_at) DESC LIMIT 1",
                    params![topic_key, fields.project(), fields.scope()],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if let Some(id) = existing {
                // Everything the replaced revision held, read in one go and
                // before the UPDATE overwrites it: the snapshot keeps the old
                // title and body, and the reply reports the size of the body it
                // replaced. Neither can be read back afterwards.
                let (
                    previous_sync_id,
                    previous_kind,
                    previous_title,
                    previous_content,
                    previous_revision,
                ): (String, String, String, String, i64) = tx.query_row(
                    "SELECT sync_id, type, title, content, revision_count
                           FROM observations WHERE id = ?1",
                    [id],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?.unwrap_or_default(),
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )?;
                // The count moves for every save under the key, but a version
                // is kept only when the text actually changes — a re-save of
                // the same words is not a previous version anybody lost.
                let content_changed =
                    previous_title != fields.title() || previous_content != fields.content();
                let replaced_at = content_changed.then(crate::timestamp::now);
                if let Some(replaced_at) = &replaced_at {
                    snapshot_observation_version_tx(
                        &tx,
                        &previous_sync_id,
                        previous_revision,
                        &previous_title,
                        &previous_content,
                        replaced_at,
                    )?;
                }
                tx.execute(
                    "UPDATE observations SET type = ?1, title = ?2, content = ?3, tool_name = ?4,
                     topic_key = ?5, normalized_hash = ?6, revision_count = revision_count + 1,
                     last_seen_at = datetime('now'), updated_at = datetime('now') WHERE id = ?7",
                    params![
                        fields.kind(),
                        fields.title(),
                        fields.content(),
                        input.tool_name,
                        topic_key,
                        fields.hash(),
                        id
                    ],
                )?;
                reschedule_review(&tx, id, fields.kind(), Some(&previous_kind))?;
                let observation = get_observation_row(&tx, id)?;
                if let Some(replaced_at) = &replaced_at {
                    // The version follows the observation's own enrolment:
                    // whatever project would carry the memory carries what it
                    // replaced.
                    enqueue_observation_version(
                        &tx,
                        &observation.sync_id,
                        previous_revision,
                        &previous_title,
                        &previous_content,
                        replaced_at,
                        observation.project.as_deref().unwrap_or_default(),
                    )?;
                }
                enqueue_observation(&tx, &observation)?;
                tx.commit()?;
                self.embed_written(&[observation.id]);
                let replaced = replaced_at.map(|_| ReplacedContent {
                    bytes: previous_content.len(),
                    shrunk: crate::memory::model::content_shrank(
                        previous_content.len(),
                        fields.content().len(),
                    ),
                });
                return Ok(AddOutcome {
                    kind: AddOutcomeKind::Revised,
                    observation,
                    replaced,
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
                params![
                    fields.hash(),
                    fields.project(),
                    fields.scope(),
                    fields.kind(),
                    fields.title(),
                    modifier.as_ref()
                ],
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
                replaced: None,
            });
        }

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
        let observation = insert_observation_tx(
            &tx,
            &input.session_id,
            input.tool_name.as_deref(),
            prompt_sync_id,
            fields,
        )?;
        tx.commit()?;
        self.embed_written(&[observation.id]);
        Ok(AddOutcome {
            kind: AddOutcomeKind::Inserted,
            observation,
            replaced: None,
        })
    }

    /// Replaces several memories with one, recording a judged `supersedes`
    /// relation to each source, in a single transaction.
    ///
    /// Engram's plan (#242) soft-deletes the sources and inserts a replacement.
    /// Ours keeps the graph Leteo already has: the replacement is inserted once,
    /// and each source gets a judged `supersedes` relation pointing at it. That
    /// makes the merge traceable — the relation names both ends — and
    /// reversible, because a re-verdict or a removal restores the source to
    /// search and context. Nothing is deleted.
    ///
    /// One transaction around the whole thing, so criterion one holds: a source
    /// in the wrong project, or one that does not exist, refuses before the
    /// replacement row or any relation is written. Every source is read and
    /// checked before the first write, so the failure the caller sees is the
    /// one that stopped it rather than a half-merge.
    pub fn consolidate_observations(
        &mut self,
        input: ConsolidateObservations,
    ) -> Result<ConsolidateOutcome, StoreError> {
        // A merge of nothing, and a merge that would record two relations to
        // one memory, are caller mistakes rather than store ones. Refused
        // before the transaction so nothing is opened for them.
        if input.source_ids.is_empty() {
            return Err(StoreError::ConsolidationSources {
                reason: "source_ids is empty; a merge needs at least one memory to replace"
                    .to_owned(),
            });
        }
        let mut seen = BTreeSet::new();
        for id in &input.source_ids {
            if !seen.insert(*id) {
                return Err(StoreError::ConsolidationSources {
                    reason: format!("source_ids repeats observation {id}"),
                });
            }
        }

        // Read before the transaction, so the borrow of `self` for the budget
        // and the borrow for the write do not overlap.
        let max_length = self.config.max_observation_length;
        let tx = self.write_transaction()?;
        ensure_session_tx(&tx, &input.session_id)?;
        // Every source, before any write: the replacement's project depends on
        // them, and a refusal must leave the store exactly as it was.
        let mut sources = Vec::with_capacity(input.source_ids.len());
        for id in &input.source_ids {
            let source = get_active_observation(&tx, *id)?;
            assert_expected_project(
                *id,
                source.project.as_deref(),
                input.expected_project.as_deref(),
            )?;
            sources.push(source);
        }
        // Where the replacement is filed. A caller that named a project means
        // it; one acting on ids alone — the CLI — inherits the project of the
        // first source, so a merge does not move the memories to whatever
        // directory it happened to be run from.
        let project = input
            .project
            .clone()
            .or_else(|| sources.first().and_then(|source| source.project.clone()));
        let fields = normalize::fields(
            &input.kind,
            &input.title,
            &input.content,
            project.as_deref(),
            &input.scope,
            input.topic_key.as_deref(),
            max_length,
        );
        if let Some(refusal) = crate::memory::rules::refuse(fields.title(), fields.content()) {
            return Err(invalid_parameter(refusal.message()));
        }
        let observation = insert_observation_tx(
            &tx,
            &input.session_id,
            input.tool_name.as_deref(),
            None,
            fields,
        )?;
        // One judged `supersedes` per source, through the same path every other
        // judged relation takes: the cross-project guard, the provenance and the
        // replication journal are not re-decided here.
        let mut relations = Vec::with_capacity(sources.len());
        for source in &sources {
            relations.push(super::relations::judge_relation_tx(
                &tx,
                &observation.sync_id,
                &source.sync_id,
                RELATION_SUPERSEDES,
                None,
                None,
                None,
            )?);
        }
        tx.commit()?;
        self.embed_written(&[observation.id]);
        Ok(ConsolidateOutcome {
            observation,
            relations,
            sources: input.source_ids,
        })
    }

    pub fn get_observation(&self, id: i64) -> Result<Observation, StoreError> {
        get_observation_row(&self.connection, id)
    }

    /// The titles and bodies a later write replaced, newest first.
    ///
    /// Keyed by the observation's `sync_id` rather than by its local `id`: a
    /// version is stored against the identifier that survives replication, so
    /// the read does not depend on which machine wrote the row. Bytes come back
    /// exactly as stored; a caller wanting the live memory reads
    /// [`Store::get_observation`].
    pub fn observation_versions(
        &self,
        observation_sync_id: &str,
    ) -> Result<Vec<ObservationVersion>, StoreError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_VERSION_COLUMNS} FROM observation_versions
              WHERE observation_sync_id = ?1
              ORDER BY revision DESC"
        ))?;
        let rows = statement.query_map([observation_sync_id], map_observation_version)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Revises a memory, returning it alone.
    ///
    /// The caller that has to report what was replaced — `mem_update`, whose
    /// reply names the size of the body it overwrote — takes
    /// [`update_observation_with_replaced`](Self::update_observation_with_replaced)
    /// instead; everything else reads the observation and no more.
    pub fn update_observation(
        &mut self,
        id: i64,
        expected_project: Option<&str>,
        input: UpdateObservation,
    ) -> Result<Observation, StoreError> {
        self.update_observation_with_replaced(id, expected_project, input)
            .map(|outcome| outcome.observation)
    }

    /// Revises a memory and reports the text the revision replaced.
    ///
    /// The read of the old title and body happens inside the write transaction,
    /// so a concurrent writer cannot slip between what is replaced and what is
    /// recorded as the version of it.
    pub fn update_observation_with_replaced(
        &mut self,
        id: i64,
        expected_project: Option<&str>,
        input: UpdateObservation,
    ) -> Result<UpdateOutcome, StoreError> {
        let max_length = self.config.max_observation_length;
        let tx = self.write_transaction()?;
        let current = get_active_observation(&tx, id)?;
        assert_expected_project(id, current.project.as_deref(), expected_project)?;
        // The text before the update, for the snapshot and the reply alike:
        // once the UPDATE has run the row holds the new words and there is no
        // second read that gets the old ones back.
        let previous_title = current.title.clone();
        let previous_content = current.content.clone();
        let previous_revision = current.revision_count;

        // A partial edit names one span of the stored body, and every way it
        // can fail to name exactly one is refused before anything is written.
        // The count is taken against `current.content` — the row this
        // transaction just read — so what is replaced is what was counted.
        if input.find.is_none() && input.replace.is_some() {
            return Err(invalid_parameter(
                "replace needs find: name the span to replace, or write the whole body with content",
            ));
        }
        if let Some(find) = input.find.as_deref() {
            if find.is_empty() {
                return Err(invalid_parameter("find cannot be empty"));
            }
            if input.content.is_some() {
                return Err(invalid_parameter(
                    "find and content are two ways to write the body; pass one",
                ));
            }
            match current.content.matches(find).count() {
                0 => return Err(StoreError::EditNotFound),
                1 => {}
                matches => return Err(StoreError::EditAmbiguous { matches }),
            }
        }

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
        // The body this write stores, and — for a find/replace — the length the
        // storage bound saw before it cut. The caller never held the edited
        // text, so this is the only place that length exists; a full-body write
        // measures its own cut from the text it sent.
        let mut edited_cut = None;
        let content = if let Some(find) = input.find.as_deref() {
            let edited = current
                .content
                .replacen(find, input.replace.as_deref().unwrap_or(""), 1);
            let stripped = normalize::strip_private(&edited);
            edited_cut = (stripped.len() > max_length).then_some(stripped.len());
            normalize::truncate_content(stripped, max_length)
        } else {
            input
                .content
                .map(|value| {
                    normalize::truncate_content(normalize::strip_private(&value), max_length)
                })
                .unwrap_or(current.content)
        };
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

        // A metadata-only change is out of the history's scope: nothing a
        // reader could call the previous version was lost when only the type,
        // project, scope or topic key moved. The title or the body is what a
        // version keeps.
        let content_changed = previous_title != title || previous_content != content;
        let replaced_at = content_changed.then(crate::timestamp::now);
        if let Some(replaced_at) = &replaced_at {
            snapshot_observation_version_tx(
                &tx,
                &current.sync_id,
                previous_revision,
                &previous_title,
                &previous_content,
                replaced_at,
            )?;
        }

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
        if let Some(replaced_at) = &replaced_at {
            enqueue_observation_version(
                &tx,
                &observation.sync_id,
                previous_revision,
                &previous_title,
                &previous_content,
                replaced_at,
                observation.project.as_deref().unwrap_or_default(),
            )?;
        }
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
        self.embed_written(&[id]);
        let replaced = replaced_at.map(|_| ReplacedContent {
            bytes: previous_content.len(),
            shrunk: crate::memory::model::content_shrank(previous_content.len(), content.len()),
        });
        Ok(UpdateOutcome {
            observation,
            replaced,
            edited_cut,
        })
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
    /// everything current and leaves the caller to drop what it cannot use,
    /// which meant asking for four times the budget and hoping: pinned memories
    /// are listed separately, session summaries are folded onto their sessions,
    /// and a narrowed scope is filtered afterwards. Four times over is a guess,
    /// and
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
        let not_superseded = super::relations::not_superseded();
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM observations o
             WHERE o.deleted_at IS NULL AND o.pinned = 0 AND o.type <> '{summary}'
               AND {not_superseded}{}
             ORDER BY datetime(o.created_at) DESC, o.id DESC LIMIT ?{limit}",
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
        let not_superseded = super::relations::not_superseded();
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM (
                 SELECT o.*, ROW_NUMBER() OVER (
                            PARTITION BY o.session_id
                            ORDER BY datetime(o.created_at) DESC, o.id DESC
                        ) AS place
                   FROM observations o
                  WHERE o.deleted_at IS NULL AND o.type = '{summary}'
                    AND o.session_id IN ({holes})
                    AND {not_superseded}
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
        // The sibling of `recent_memories`, and it listed a memory a later one
        // had overtaken: the CLI `recent` command, the Obsidian view and the
        // conflict scan all read this door. The lossless JSON export does not —
        // it reads the tables directly — so nothing here reaches a backup.
        let not_superseded = super::relations::not_superseded();
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS} FROM observations o
             WHERE o.deleted_at IS NULL AND {not_superseded}{without}{}
             ORDER BY datetime(o.created_at) DESC, o.id DESC LIMIT ?{limit}",
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
        let not_superseded = super::relations::not_superseded();
        if fts.is_empty() {
            let (clause, values) = Self::project_clause(projects, "project");
            let bound: Vec<&dyn rusqlite::ToSql> =
                values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
            let total = self.connection.query_row(
                &format!(
                    "SELECT COUNT(*) FROM observations o
                     WHERE o.deleted_at IS NULL AND {not_superseded}{clause}"
                ),
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
                 WHERE observations_fts MATCH ? AND o.deleted_at IS NULL
                   AND {not_superseded}{clause}"
            ),
            bound.as_slice(),
            |row| row.get(0),
        )?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {OBSERVATION_COLUMNS_JOINED}
             FROM observations_fts fts CROSS JOIN observations o ON o.id = fts.rowid
             WHERE observations_fts MATCH ? AND o.deleted_at IS NULL
               AND {not_superseded}{clause}
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

    pub fn pin_observation(
        &mut self,
        id: i64,
        expected_project: Option<&str>,
    ) -> Result<(), StoreError> {
        self.set_observation_pinned(id, expected_project, true)
    }

    pub fn unpin_observation(
        &mut self,
        id: i64,
        expected_project: Option<&str>,
    ) -> Result<(), StoreError> {
        self.set_observation_pinned(id, expected_project, false)
    }

    fn set_observation_pinned(
        &mut self,
        id: i64,
        expected_project: Option<&str>,
        pinned: bool,
    ) -> Result<(), StoreError> {
        let tx = self.write_transaction()?;
        // The row is read rather than trusted to the UPDATE's row count: the
        // count can say a row is absent but not which project it is in, and the
        // ownership check needs the project. Inside one immediate transaction
        // the row cannot move between this read and the write below.
        let observation = get_active_observation(&tx, id)?;
        assert_expected_project(id, observation.project.as_deref(), expected_project)?;
        tx.execute(
            "UPDATE observations SET pinned = ?1 WHERE id = ?2 AND deleted_at IS NULL",
            params![pinned, id],
        )?;
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

    pub fn delete_observation(
        &mut self,
        id: i64,
        expected_project: Option<&str>,
        hard_delete: bool,
    ) -> Result<(), StoreError> {
        let tx = self.write_transaction()?;
        let observation = if hard_delete {
            get_observation_row(&tx, id)?
        } else {
            get_active_observation(&tx, id)?
        };
        assert_expected_project(id, observation.project.as_deref(), expected_project)?;
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

    /// Stores the learnings one subagent left, reporting what got in before a
    /// failure stopped it.
    ///
    /// Each learning is its own transaction, so a capture that meets a busy
    /// store part way through has already committed the learnings before the
    /// one that failed. Returning only the error threw that away, and the
    /// message the hook printed then said the whole text was gone when part of
    /// it was in the store — see [`StoreError::capture_lost`].
    pub fn passive_capture(
        &mut self,
        input: PassiveCapture,
    ) -> Result<PassiveCaptureResult, CaptureFailure> {
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
            let duplicate = match self.connection.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM observations WHERE normalized_hash = ?1
                      AND ifnull(project, '') = ?2 AND deleted_at IS NULL
                 )",
                params![hash, project],
                |row| row.get::<_, bool>(0),
            ) {
                Ok(duplicate) => duplicate,
                Err(error) => {
                    return Err(CaptureFailure {
                        error: error.into(),
                        partial: result,
                    });
                }
            };
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
            let outcome = match self.add_observation(AddObservation {
                session_id: input.session_id.clone(),
                kind: "passive".to_owned(),
                title,
                content: learning,
                tool_name: normalize::optional(Some(&input.source)),
                project: (!project.is_empty()).then_some(project.clone()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            }) {
                Ok(outcome) => outcome,
                Err(error) => {
                    return Err(CaptureFailure {
                        error,
                        partial: result,
                    });
                }
            };
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
