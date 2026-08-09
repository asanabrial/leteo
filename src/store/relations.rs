//! How two memories relate — conflicts, supersessions, judgments.

use super::*;

/// Which relations a listing is about: the joins that name both ends, and the
/// three filters — `?1` project, `?2` status, `?3` since.
///
/// The count and the page it paginates have to describe the same set. They were
/// written out twice, with the same three parameters in the same order, and a
/// filter changed in one and not the other makes the total disagree with what
/// the rows can produce — a pager offering a page the list cannot fill.
const RELATION_LISTING_SOURCE: &str = "FROM memory_relations r
             LEFT JOIN observations src
               ON src.sync_id = r.source_id AND src.deleted_at IS NULL
             LEFT JOIN observations tgt
               ON tgt.sync_id = r.target_id AND tgt.deleted_at IS NULL
             WHERE (?1 = '' OR ifnull(src.project, '') = ?1 OR ifnull(tgt.project, '') = ?1)
               AND (?2 = '' OR r.judgment_status = ?2)
               AND (?3 IS NULL OR datetime(r.created_at) >= datetime(?3))";

/// The extra conditions that make a pending pair one `mem_judge` will accept.
///
/// Appended to [`RELATION_LISTING_SOURCE`], never instead of it, so the set is
/// the same set narrowed further.
///
/// Two ways a pair can be impossible to rule on, and both were measured against
/// the store rather than reasoned about, because reasoning got one of them
/// backwards. `judge_relation` runs `validate_cross_project_guard`, which
/// refuses when a memory is *absent from the table* and when the two ends carry
/// different projects. A **soft**-deleted memory keeps its row, so that pair
/// judges perfectly well — the first draft of this told the agent it could not
/// be ruled on, which is false, and parked it for good on the strength of the
/// claim.
///
/// Filtered out here rather than labelled and offered. These are ordered oldest
/// first so that nothing starves, and a pair no call can ever settle would sit
/// at the head of that queue for good, holding a slot against work that can be
/// done — the property the ordering exists for, inverted.
///
/// Both halves are a last line rather than the fix, and it is worth knowing
/// which. A hard deletion already marks its relations `orphaned`, so the two
/// `EXISTS` clauses cover a state no current path reaches — measured, and
/// `soft_and_hard_delete_are_journaled_and_orphan_relations` is the guard that
/// keeps it that way. They stay because that marking lives at four call sites,
/// and a fifth one that forgets is exactly what `orphan_relations_tx` was
/// extracted to make unlikely rather than impossible. The cross-project clause
/// is now handled at the source too — `strand_relations_tx` retires those the
/// moment a memory changes project — and it stays for the rows stranded before
/// that existed, which no migration goes back to clean.
const RELATION_JUDGEABLE: &str = "
               AND EXISTS(SELECT 1 FROM observations o WHERE o.sync_id = r.source_id)
               AND EXISTS(SELECT 1 FROM observations o WHERE o.sync_id = r.target_id)
               AND NOT EXISTS(
                     SELECT 1 FROM observations a, observations b
                      WHERE a.sync_id = r.source_id AND b.sync_id = r.target_id
                        AND ifnull(a.project, '') <> '' AND ifnull(b.project, '') <> ''
                        AND a.project <> b.project)";

impl Store {
    /// Whether somebody has already ruled on this pair.
    ///
    /// `find_candidates` hides settled pairs when it is going to file one, and
    /// turns that off with `skip_insert` — which is right for a preview asking
    /// what is *there*, and is why the two callers that preview have to ask
    /// this for themselves. `scan_project` does, and counts them as
    /// `already_related`; the semantic scan did not, so it paid a model call
    /// for a question the store had already answered and wrote the model's
    /// answer over the one on record.
    pub fn pair_is_judged(&self, source_id: &str, target_id: &str) -> Result<bool, StoreError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM memory_relations
                  WHERE judgment_status = 'judged'
                    AND ((source_id = ?1 AND target_id = ?2)
                      OR (source_id = ?2 AND target_id = ?1))
             )",
            params![source_id, target_id],
            |row| row.get::<_, bool>(0),
        )?)
    }

    pub fn find_candidates(
        &mut self,
        saved_id: i64,
        options: CandidateOptions,
    ) -> Result<Vec<Candidate>, StoreError> {
        let limit = options.limit.unwrap_or(3).max(1);
        // This floor gates almost nothing, and the number cannot be fixed by
        // choosing a different one. bm25 grows with how many terms the query
        // has and how rare they are, so it is not comparable between queries:
        // driving the built binary over the protocol, `Session summary: leteo`
        // scored its best candidate at -8.3 and a ten-word title scored -93.3.
        // Both clear -2.0 without effort. Replaying the query over 400 real
        // memories, 399 of them get the full three proposals.
        //
        // A gate that meant the same thing for both would have to be relative
        // to the query — the median margin `nearest_observations` uses. What is
        // missing is any way to choose the margin: the pairs an agent judged
        // were proposed by this finder while it compared the floor backwards,
        // so they are a record of the bug rather than of what is worth
        // proposing, and `topic_key` cannot stand in for a label because it is
        // very nearly unique — 2,071 distinct keys across 2,077 memories that
        // have one, and five families with more than a single member.
        //
        // Left alone deliberately. Until the fixed finder has produced verdicts
        // of its own there is nothing to measure a margin against, and a
        // constant chosen without one is a guess wearing a decimal point.
        let floor = options.bm25_floor.unwrap_or(-2.0);
        let (title, stored_project, stored_scope, source_sync_id) = self
            .connection
            .query_row(
                "SELECT title, ifnull(project, ''), scope, ifnull(sync_id, '')
                 FROM observations WHERE id = ?1",
                [saved_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::ObservationNotFound(saved_id))?;
        let project = options
            .project
            .as_deref()
            .map(normalize::project)
            .unwrap_or(stored_project);
        let scope = options
            .scope
            .as_deref()
            .map(normalize::scope)
            .map(str::to_owned)
            .unwrap_or(stored_scope);
        let fts_query = candidate_fts_query(&title);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }
        // Narrowed to the project inside the index as well as after the join.
        // A conflict is looked for on every single save, and it was looked for
        // against every memory of every project before the `WHERE` threw the
        // other projects away: 6.9ms a save against 1.5ms, on a copy of a real
        // store.
        //
        // Free only because the project column is weighted 0.0 below, and that
        // is not a detail. Measured over 600 real memories, narrowing while the
        // column still counted changed which candidates were proposed for 134
        // of them: matching one more term shifts a document's score by an
        // amount that depends on its length, and the floor here is absolute
        // rather than relative to the other candidates. With the weight at
        // zero, narrowed and not narrowed return the same candidates for all
        // 600. See `normalize::fts_within_project`.
        let fts_query = normalize::fts_within_project(&fts_query, &project).unwrap_or(fts_query);

        // A pair somebody has already been asked about is not a candidate.
        //
        // Only where this is going to file one. A preview — `scan_project`,
        // the semantic scan — is asking what is *there*, and hiding the judged
        // pairs from it would turn "five candidates, five already related" into
        // "nothing to look at", which is a different lie. What must not happen
        // is a second row for a question already asked: a memory saved again
        // comes back through the dedupe path with the same id, this runs on it
        // again, and the pair is proposed afresh. Fourteen pairs on a real
        // store already carried more than one row, one of them four.
        //
        // In the query rather than in the loop below, so a settled pair does
        // not spend one of the places the limit allows.
        //
        // Either direction, because the relation is symmetric: `A supersedes B`
        // and `B supersedes A` are answers to one question.
        //
        // Null for a preview, which is what switches the clause off — one
        // statement and one shape, with the `OR` short-circuiting before the
        // subquery is evaluated at all.
        let settled = (!options.skip_insert).then_some(source_sync_id.as_str());
        let raw = {
            // The project column is weighted out of the score, which is what
            // every other ranking in this codebase already does — search, the
            // prompt hint and the topic-key lookup all pass `0.0` for it. This
            // one call had it counting at full weight, and the search it scores
            // is already restricted to a single project: the column holds the
            // same value for every candidate, so it cannot tell them apart,
            // while still moving each one by a different amount according to
            // its length. A title that says the project's name — 328 of 600 on
            // a real store, `Session summary: leteo` and every Spanish title
            // that mentions it — matched that column on every candidate at
            // once. Which candidates are proposed changes for 84 of those 600;
            // there is no label that says whether they are better, and the only
            // one available (pairs somebody has already judged) is circular,
            // since this same finder proposed them.
            // A session summary is not a candidate for anything.
            //
            // It narrates what a session did; it does not claim anything about
            // the project. Nothing supersedes it and it conflicts with nothing,
            // so every verdict `mem_judge` offers is the wrong shape for it and
            // the agent's only honest answer is `not_conflict` — a round trip
            // and a stored row to say nothing. `nearest_observations` leaves
            // them out of the third search stage for the same reason.
            //
            // They are a quarter of this store — 901 live rows of 3,769 — and
            // they all carry a session's worth of text, so they match on
            // whatever words a title happens to share. Over the 400 most recent
            // memories, replayed through this query: 106 of the 1,197 proposals
            // were a summary, and 54 of the 399 saves that got any proposal at
            // all got one.
            let summary = crate::memory::model::SESSION_SUMMARY;
            let mut statement = self.connection.prepare(&format!(
                "SELECT o.id, ifnull(o.sync_id, ''), o.title, o.type, o.topic_key,
                        bm25(observations_fts, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0) AS score
                 FROM observations_fts fts
                 CROSS JOIN observations o ON o.id = fts.rowid
                 WHERE observations_fts MATCH ?1
                   AND o.id != ?2
                   AND o.deleted_at IS NULL
                   AND o.type <> '{summary}'
                   AND ifnull(o.project, '') = ?3
                   AND o.scope = ?4
                   AND (?6 IS NULL OR NOT EXISTS (
                         SELECT 1 FROM memory_relations r
                          WHERE (r.source_id = ?6 AND r.target_id = o.sync_id)
                             OR (r.source_id = o.sync_id AND r.target_id = ?6)
                       ))
                 ORDER BY score
                 LIMIT ?5"
            ))?;
            let rows = statement.query_map(
                params![
                    fts_query,
                    saved_id,
                    project,
                    scope,
                    // Wide enough to have a median, which is what the margin
                    // below is measured against — the same sample
                    // `nearest_observations` takes for the same reason.
                    RECALL_SAMPLE as i64,
                    settled
                ],
                |row| {
                    Ok(Candidate {
                        id: row.get(0)?,
                        sync_id: row.get(1)?,
                        title: row.get(2)?,
                        kind: row.get(3)?,
                        topic_key: row.get(4)?,
                        score: row.get(5)?,
                        judgment_id: String::new(),
                    })
                },
            )?;
            let mut scored = Vec::new();
            for row in rows {
                scored.push(row?);
            }
            // Better than the ordinary match for *this* query, not better than
            // a number. See `CANDIDATE_MARGIN`, where the measurement is.
            //
            // Only where the sample filled up, which is how this knows there
            // is an ordinary to be better than.
            //
            // Not "at least three", which is the bar `nearest_observations`
            // sets: a median needs a background, and where every match is as
            // good as every other the median sits on top of the best one and
            // the margin throws away the lot. That is the shape of a small
            // store — three revisions of one memory and nothing else — and it
            // is exactly where a proposal is worth most, so the suite caught it
            // as soon as the margin went in. A query that matched fewer
            // memories than the sample asked for has not shown this what an
            // ordinary match looks like, and there the absolute floor is the
            // whole gate, as it was everywhere before.
            if scored.len() >= RECALL_SAMPLE {
                let mut ranks: Vec<f64> = scored.iter().map(|candidate| candidate.score).collect();
                ranks.sort_by(f64::total_cmp);
                let median = ranks[ranks.len() / 2];
                scored.retain(|candidate| candidate.score <= median * CANDIDATE_MARGIN);
            }
            let mut candidates = Vec::new();
            for candidate in scored {
                // At least as good as the floor. bm25 in SQLite is negative and
                // more negative is better, so `-2.0` is the weakest match worth
                // anybody's attention — and comparing it the other way round
                // kept exactly the matches nobody wants: the ones that barely
                // match at all, everything genuinely close being thrown away
                // for scoring too well.
                if candidate.score <= floor {
                    candidates.push(candidate);
                    if candidates.len() == limit {
                        break;
                    }
                }
            }
            candidates
        };

        if options.skip_insert || raw.is_empty() {
            return Ok(raw);
        }

        let tx = self.write_transaction()?;
        let mut candidates = Vec::with_capacity(raw.len());
        for mut candidate in raw {
            let judgment_id = normalize::sync_id("rel");
            if tx
                .execute(
                    "INSERT INTO memory_relations
                     (sync_id, source_id, target_id, relation, judgment_status,
                      created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'pending', 'pending', datetime('now'), datetime('now'))",
                    params![judgment_id, source_sync_id, candidate.sync_id],
                )
                .is_ok()
            {
                candidate.judgment_id = judgment_id;
                candidates.push(candidate);
            }
        }
        tx.commit()?;
        Ok(candidates)
    }

    pub fn save_relation(&mut self, input: SaveRelationParams) -> Result<Relation, StoreError> {
        self.connection.execute(
            "INSERT INTO memory_relations
             (sync_id, source_id, target_id, relation, judgment_status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'pending', 'pending', datetime('now'), datetime('now'))",
            params![input.sync_id, input.source_id, input.target_id],
        )?;
        self.get_relation(&input.sync_id)
    }

    pub fn get_relation(&self, sync_id: &str) -> Result<Relation, StoreError> {
        self.connection
            .query_row(
                &format!("SELECT {RELATION_COLUMNS} FROM memory_relations WHERE sync_id = ?1"),
                [sync_id],
                map_relation,
            )
            .optional()?
            .ok_or_else(|| StoreError::RelationNotFound(sync_id.to_owned()))
    }

    pub fn get_relation_by_id(&self, id: i64) -> Result<RelationListItem, StoreError> {
        self.connection
            .query_row(
                "SELECT r.id, r.sync_id, r.relation, r.judgment_status,
                        ifnull(r.source_id, ''), ifnull(src.title, ''),
                        ifnull(r.target_id, ''), ifnull(tgt.title, ''),
                        r.created_at, r.updated_at
                 FROM memory_relations r
                 LEFT JOIN observations src
                   ON src.sync_id = r.source_id AND src.deleted_at IS NULL
                 LEFT JOIN observations tgt
                   ON tgt.sync_id = r.target_id AND tgt.deleted_at IS NULL
                 WHERE r.id = ?1",
                [id],
                map_relation_list_item,
            )
            .optional()?
            .ok_or_else(|| StoreError::RelationNotFound(id.to_string()))
    }

    pub fn judge_relation(&mut self, input: JudgeRelationParams) -> Result<Relation, StoreError> {
        validate_relation_verb(&input.relation)?;
        validate_optional_confidence(input.confidence)?;
        // Free text from an agent, held to the rules the rest of the store's
        // text is: see `normalize::judgment_text`. Before the transaction, so
        // the borrow of `self` for the budget and the borrow for the write do
        // not overlap.
        let max = self.config.max_observation_length;
        let reason = normalize::judgment_text(input.reason.as_deref(), max);
        let evidence = normalize::judgment_text(input.evidence.as_deref(), max);

        let tx = self.write_transaction()?;
        let current = get_relation_tx(&tx, &input.judgment_id)?;
        let (source_project, target_project) =
            validate_cross_project_guard(&tx, &current.source_id, &current.target_id)?;
        let marked_by_model = input
            .marked_by_model
            .filter(|value| !value.trim().is_empty());
        let session_id = input.session_id.filter(|value| !value.trim().is_empty());
        tx.execute(
            "UPDATE memory_relations
             SET relation = ?1, reason = ?2, evidence = ?3, confidence = ?4,
                 judgment_status = 'judged', marked_by_actor = ?5,
                 marked_by_kind = ?6, marked_by_model = ?7, session_id = ?8,
                 updated_at = datetime('now')
             WHERE sync_id = ?9",
            params![
                input.relation,
                reason,
                evidence,
                input.confidence,
                input.marked_by_actor,
                input.marked_by_kind,
                marked_by_model,
                session_id,
                input.judgment_id
            ],
        )?;
        let relation = get_relation_tx(&tx, &current.sync_id)?;
        enqueue_relation_if_enrolled(&tx, &relation, &source_project, &target_project)?;
        tx.commit()?;
        Ok(relation)
    }

    pub fn judge_by_semantic(
        &mut self,
        input: JudgeBySemanticParams,
    ) -> Result<String, StoreError> {
        if input.source_id.trim().is_empty() {
            return Err(invalid_parameter("source_id is required"));
        }
        if input.target_id.trim().is_empty() {
            return Err(invalid_parameter("target_id is required"));
        }
        validate_relation_verb(&input.relation)?;
        validate_optional_confidence(input.confidence)?;
        if input.relation == RELATION_NOT_CONFLICT {
            return Ok(String::new());
        }

        // The same door `judge_relation` goes through, and it went through it
        // alone. `<private>` is stripped from a memory's title and body, from a
        // prompt, from a session summary, from a passive capture, and from the
        // reason and evidence of a *manual* verdict — and a semantic one wrote
        // its `reasoning` into the same column with the markers intact. Driven
        // with a secret in every write door at once, this was the one row that
        // came back holding it.
        //
        // Bounded here too, for the reason the manual one is: this is text a
        // caller wrote and the column is read back into replies.
        let reasoning = normalize::judgment_text(
            input.reasoning.as_deref(),
            self.config.max_observation_length,
        );

        let tx = self.write_transaction()?;
        let (source_project, target_project) =
            validate_cross_project_guard(&tx, &input.source_id, &input.target_id)?;
        let existing = tx
            .query_row(
                "SELECT sync_id FROM memory_relations
                 WHERE (source_id = ?1 AND target_id = ?2)
                    OR (source_id = ?2 AND target_id = ?1)
                 ORDER BY id LIMIT 1",
                params![input.source_id, input.target_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let sync_id = existing.unwrap_or_else(|| normalize::sync_id("rel"));
        let model = input.model.filter(|value| !value.trim().is_empty());
        if get_relation_tx_optional(&tx, &sync_id)?.is_some() {
            tx.execute(
                "UPDATE memory_relations
                 SET relation = ?1, judgment_status = 'judged', confidence = ?2,
                     reason = ?3, marked_by_actor = 'leteo', marked_by_kind = 'system',
                     marked_by_model = ?4, updated_at = datetime('now')
                 WHERE sync_id = ?5",
                params![input.relation, input.confidence, reasoning, model, sync_id],
            )?;
        } else {
            tx.execute(
                "INSERT INTO memory_relations
                 (sync_id, source_id, target_id, relation, judgment_status, confidence,
                  reason, marked_by_actor, marked_by_kind, marked_by_model,
                  created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'judged', ?5, ?6, 'leteo', 'system', ?7,
                         datetime('now'), datetime('now'))",
                params![
                    sync_id,
                    input.source_id,
                    input.target_id,
                    input.relation,
                    input.confidence,
                    reasoning,
                    model
                ],
            )?;
        }
        let relation = get_relation_tx(&tx, &sync_id)?;
        enqueue_relation_if_enrolled(&tx, &relation, &source_project, &target_project)?;
        tx.commit()?;
        Ok(sync_id)
    }

    pub fn list_relations(
        &self,
        options: ListRelationsOptions,
    ) -> Result<Vec<RelationListItem>, StoreError> {
        let project = options
            .project
            .as_deref()
            .map(normalize::project)
            .unwrap_or_default();
        let status = options.status.unwrap_or_default();
        let limit = options.limit.unwrap_or(usize::MAX).min(i64::MAX as usize) as i64;
        let offset = options.offset.min(i64::MAX as usize) as i64;
        let mut statement = self.connection.prepare(&format!(
            "SELECT r.id, r.sync_id, r.relation, r.judgment_status,
                    ifnull(r.source_id, ''), ifnull(src.title, ''),
                    ifnull(r.target_id, ''), ifnull(tgt.title, ''),
                    r.created_at, r.updated_at
             {RELATION_LISTING_SOURCE}
             ORDER BY datetime(r.created_at) DESC, r.id DESC
             LIMIT ?4 OFFSET ?5"
        ))?;
        let rows = statement.query_map(
            params![project, status, options.since, limit, offset],
            map_relation_list_item,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The pairs that have been waiting longest and that `mem_judge` will
    /// accept, with enough of both memories to rule on them.
    ///
    /// Oldest first, and the ordering is the point rather than a preference. A
    /// session opening hands over a few of these; newest-first would offer the
    /// same recent pairs every time while the ones already forgotten stayed
    /// forgotten, which is how the oldest on a real store reached eight weeks.
    /// Ordered this way, a pair not ruled on today is nearer the front tomorrow,
    /// so no pair can starve — which only holds because [`RELATION_JUDGEABLE`]
    /// keeps out the ones that could never be settled. One of those at the head
    /// of a queue ordered by age stays there.
    ///
    /// Shares [`RELATION_LISTING_SOURCE`] with `count_relations` for the reason
    /// that constant exists, and is narrowed past it, so the two no longer
    /// describe the same set. [`Store::count_pending_judgeable`] is the count
    /// that matches this list; the difference between them is what the opening
    /// block reports as pairs nothing can do anything about.
    pub fn pending_pairs(
        &self,
        project: &str,
        limit: usize,
    ) -> Result<Vec<PendingPair>, StoreError> {
        let project = normalize::project(project);
        let limit = limit.min(i64::MAX as usize) as i64;
        let mut statement = self.connection.prepare(&format!(
            "SELECT r.sync_id, r.created_at,
                    src.id, src.type, src.title, src.topic_key,
                    tgt.id, tgt.type, tgt.title, tgt.topic_key
             {RELATION_LISTING_SOURCE}{RELATION_JUDGEABLE}
             ORDER BY datetime(r.created_at) ASC, r.id ASC
             LIMIT ?4"
        ))?;
        let rows = statement.query_map(
            params![project, JUDGMENT_STATUS_PENDING, None::<String>, limit],
            |row| {
                Ok(PendingPair {
                    judgment_id: row.get(0)?,
                    created_at: row.get(1)?,
                    source: pending_side(row, 2)?,
                    target: pending_side(row, 6)?,
                })
            },
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// How many pending pairs of this project could actually be ruled on.
    ///
    /// The same narrowing [`Store::pending_pairs`] lists, counted, so the two
    /// cannot come to mean different things. Subtracted from the plain pending
    /// count it gives the pairs no `mem_judge` call will ever settle: a memory
    /// deleted outright, or two memories that ended up in different projects.
    /// Those are worth a sentence and are not worth a slot.
    pub fn count_pending_judgeable(&self, project: &str) -> Result<i64, StoreError> {
        let project = normalize::project(project);
        Ok(self.connection.query_row(
            &format!("SELECT COUNT(*) {RELATION_LISTING_SOURCE}{RELATION_JUDGEABLE}"),
            params![project, JUDGMENT_STATUS_PENDING, None::<String>],
            |row| row.get(0),
        )?)
    }

    /// What the graph says about memories that are about to be handed over.
    ///
    /// Leteo builds this graph at real cost — a lexical scan proposes pairs and
    /// a language model judges them — and until now nothing read it back on the
    /// way out. A memory that a later one has overturned looked exactly like a
    /// memory that still stands.
    ///
    /// Only judged verdicts count. A pending pair is a guess nobody has
    /// confirmed, and warning on a guess is how a hint turns into noise.
    pub fn caveats_for(
        &self,
        sync_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<Caveat>>, StoreError> {
        let mut result: BTreeMap<String, Vec<Caveat>> = BTreeMap::new();
        for chunk in sync_ids.chunks(CAVEAT_LOOKUP_CHUNK) {
            self.collect_caveats(chunk, &mut result)?;
        }
        Ok(result)
    }

    fn collect_caveats(
        &self,
        sync_ids: &[String],
        result: &mut BTreeMap<String, Vec<Caveat>>,
    ) -> Result<(), StoreError> {
        let named: BTreeSet<&String> = sync_ids.iter().collect();
        if named.is_empty() {
            return Ok(());
        }
        let placeholders = std::iter::repeat_n("?", sync_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT r.relation, r.source_id, r.target_id,
                    src.id AS source_row, src.title AS source_title,
                    tgt.id AS target_row, tgt.title AS target_title
             FROM memory_relations r
             JOIN observations src
               ON src.sync_id = r.source_id AND src.deleted_at IS NULL
             JOIN observations tgt
               ON tgt.sync_id = r.target_id AND tgt.deleted_at IS NULL
             WHERE r.judgment_status = '{JUDGMENT_STATUS_JUDGED}'
               AND r.relation IN ('{RELATION_SUPERSEDES}', '{RELATION_CONFLICTS_WITH}')
               AND (r.source_id IN ({placeholders}) OR r.target_id IN ({placeholders}))
             ORDER BY r.id"
        );
        let values = sync_ids.iter().chain(sync_ids.iter());
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            Ok((
                row.get::<_, String>("relation")?,
                row.get::<_, String>("source_id")?,
                row.get::<_, String>("target_id")?,
                row.get::<_, i64>("source_row")?,
                row.get::<_, String>("source_title")?,
                row.get::<_, i64>("target_row")?,
                row.get::<_, String>("target_title")?,
            ))
        })?;
        for row in rows {
            let (
                relation,
                source_id,
                target_id,
                source_row,
                source_title,
                target_row,
                target_title,
            ) = row?;
            // The same pair can carry more than one judged relation — a real
            // store has two saying the same thing about one memory, from two
            // scans that both found it. Saying it twice reads as two separate
            // problems.
            let mut note = |sync_id: String, caveat: Caveat| {
                let existing: &mut Vec<Caveat> = result.entry(sync_id).or_default();
                if !existing
                    .iter()
                    .any(|seen| seen.verb == caveat.verb && seen.other_id == caveat.other_id)
                {
                    existing.push(caveat);
                }
            };
            // A relation reads in one direction. "A supersedes B" leaves B
            // stale and A perfectly fine, so only the target is warned; a
            // conflict has no direction and both ends are contested.
            if relation == RELATION_SUPERSEDES {
                if named.contains(&target_id) {
                    note(
                        target_id,
                        Caveat {
                            verb: CaveatVerb::SupersededBy,
                            other_id: source_row,
                            other_title: source_title,
                        },
                    );
                }
                continue;
            }
            if named.contains(&source_id) {
                note(
                    source_id,
                    Caveat {
                        verb: CaveatVerb::ConflictsWith,
                        other_id: target_row,
                        other_title: target_title.clone(),
                    },
                );
            }
            if named.contains(&target_id) {
                note(
                    target_id,
                    Caveat {
                        verb: CaveatVerb::ConflictsWith,
                        other_id: source_row,
                        other_title: source_title,
                    },
                );
            }
        }
        Ok(())
    }

    pub fn count_relations(&self, options: ListRelationsOptions) -> Result<i64, StoreError> {
        let project = options
            .project
            .as_deref()
            .map(normalize::project)
            .unwrap_or_default();
        let status = options.status.unwrap_or_default();
        Ok(self.connection.query_row(
            &format!("SELECT COUNT(*) {RELATION_LISTING_SOURCE}"),
            params![project, status, options.since],
            |row| row.get(0),
        )?)
    }

    pub fn relation_stats(&self, project: Option<&str>) -> Result<RelationStats, StoreError> {
        let project = project.map(normalize::project).unwrap_or_default();
        let mut stats = RelationStats {
            project: project.clone(),
            ..RelationStats::default()
        };
        let mut statement = self.connection.prepare(
            "SELECT r.relation, r.judgment_status, COUNT(*)
             FROM memory_relations r
             LEFT JOIN observations src
               ON src.sync_id = r.source_id AND src.deleted_at IS NULL
             LEFT JOIN observations tgt
               ON tgt.sync_id = r.target_id AND tgt.deleted_at IS NULL
             WHERE ?1 = '' OR ifnull(src.project, '') = ?1 OR ifnull(tgt.project, '') = ?1
             GROUP BY r.relation, r.judgment_status",
        )?;
        let rows = statement.query_map([&project], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        for row in rows {
            let (relation, status, count) = row?;
            *stats.by_relation.entry(relation).or_default() += count;
            *stats.by_judgment_status.entry(status).or_default() += count;
        }
        stats.deferred = self.connection.query_row(
            "SELECT COUNT(*) FROM sync_deferred_mutations WHERE apply_status = 'deferred'",
            [],
            |row| row.get(0),
        )?;
        stats.dead = self.connection.query_row(
            "SELECT COUNT(*) FROM sync_deferred_mutations WHERE apply_status = 'dead'",
            [],
            |row| row.get(0),
        )?;
        Ok(stats)
    }
}
