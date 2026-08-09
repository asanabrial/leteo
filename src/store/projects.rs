//! Projects: naming, merging, listing and deleting a whole one.

use super::*;

impl Store {
    /// Walks a project's observations, collects lexical candidates for each of
    /// them, and optionally inserts the new pairs as pending relations.
    pub fn scan_project(&mut self, options: ScanOptions) -> Result<ScanResult, StoreError> {
        let project = normalize::project(&options.project);
        let max_insert = options.max_insert.unwrap_or(100).max(1);
        let mut result = ScanResult {
            project: project.clone(),
            dry_run: !options.apply,
            ..ScanResult::default()
        };

        let observations = {
            let mut statement = self.connection.prepare(
                "SELECT id, ifnull(sync_id, ''), scope
                 FROM observations
                 WHERE ifnull(project, '') = ?1
                   AND deleted_at IS NULL
                   AND (?2 IS NULL OR datetime(created_at) >= datetime(?2))
                 ORDER BY id",
            )?;
            let rows = statement.query_map(params![project, options.since], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        // Pairs this run has already decided on.
        //
        // The finder proposes a pair from both ends, so the same two memories
        // arrive twice. The apply used to deduplicate them by accident — the
        // first insert made the second one "already related" — and the preview,
        // writing nothing, counted both and promised one more relation than the
        // apply would create.
        //
        // Held here so the two agree, and so the apply stops asking the
        // database about a pair it decided a moment ago.
        let mut decided: BTreeSet<(String, String)> = BTreeSet::new();

        for (id, sync_id, scope) in observations {
            result.inspected += 1;
            let candidates = self.find_candidates(
                id,
                CandidateOptions {
                    project: Some(project.clone()),
                    scope: Some(scope),
                    limit: Some(10),
                    skip_insert: true,
                    ..CandidateOptions::default()
                },
            )?;
            result.candidates_found += candidates.len() as i64;

            // The dry run asks the same questions the apply does.
            //
            // It used to skip straight past this loop, so `already_related` was
            // zero by construction and `relations_inserted` was zero too — a
            // preview whose two numbers described nothing the apply would do.
            // On a real store the difference is not rounding: a scan of one
            // project previewed 2,400 candidates and 0 already related, and
            // applying it skipped 299 of them as pairs the store already knew.
            //
            // It costs the preview what the apply pays for the same answer:
            // one indexed lookup per candidate, 876 ms against 1,217. A preview
            // that is cheaper than the act by being wrong is not a preview.
            for candidate in candidates {
                if result.relations_inserted >= max_insert as i64 {
                    result.capped = true;
                    return Ok(result);
                }
                let pair = if sync_id <= candidate.sync_id {
                    (sync_id.clone(), candidate.sync_id.clone())
                } else {
                    (candidate.sync_id.clone(), sync_id.clone())
                };
                if decided.contains(&pair) {
                    result.already_related += 1;
                    continue;
                }
                let related = self.connection.query_row(
                    "SELECT EXISTS(
                         SELECT 1 FROM memory_relations
                         WHERE (source_id = ?1 AND target_id = ?2)
                            OR (source_id = ?2 AND target_id = ?1)
                     )",
                    params![sync_id, candidate.sync_id],
                    |row| row.get::<_, bool>(0),
                )?;
                if related {
                    result.already_related += 1;
                    continue;
                }
                decided.insert(pair);
                if options.apply {
                    self.save_relation(SaveRelationParams {
                        sync_id: normalize::sync_id("rel"),
                        source_id: sync_id.clone(),
                        target_id: candidate.sync_id,
                    })?;
                }
                result.relations_inserted += 1;
            }
        }
        Ok(result)
    }

    /// A `project IN (?, ?, …)` clause, and the values to bind to it.
    ///
    /// An empty selection means every project, which is expressed as the absence
    /// of a clause rather than as an empty `IN ()` — SQLite accepts that but it
    /// matches nothing, so a filter of "no projects chosen" would silently
    /// return an empty screen instead of the whole store.
    pub(super) fn project_clause(projects: &[String], column: &str) -> (String, Vec<String>) {
        let values: Vec<String> = projects
            .iter()
            .map(|project| normalize::project(project))
            .filter(|project| !project.is_empty())
            .collect();
        if values.is_empty() {
            return (String::new(), values);
        }
        // Placeholders rather than the names themselves: these arrive from the
        // caller, and a project named `x') OR 1=1 --` is a legal project name.
        let marks = vec!["?"; values.len()].join(", ");
        (format!(" AND {column} IN ({marks})"), values)
    }

    pub fn export_project(&self, project: &str) -> Result<ExportData, StoreError> {
        let project = normalize::project(project);
        if project.is_empty() {
            return Err(invalid_parameter(crate::project::EMPTY_NAME));
        }
        self.export_with_project(Some(&project))
    }

    pub(super) fn export_with_project(
        &self,
        project: Option<&str>,
    ) -> Result<ExportData, StoreError> {
        // Sessions, observations and prompts are three separate statements, and
        // a writer that commits between any two of them splits an export down
        // the middle: an observation whose session is missing, or a prompt
        // whose session was exported without it. The cloud validates those
        // references and rejects the entire chunk, so the sync fails and backs
        // off for something the client did to itself.
        //
        // A deferred transaction pins one snapshot across all three reads. It
        // takes no write lock and is rolled back on drop, so it costs a
        // concurrent writer nothing.
        let snapshot = self.connection.unchecked_transaction()?;

        let mut data = ExportData {
            version: EXPORT_FORMAT_VERSION.to_owned(),
            exported_at: sqlite_now(),
            sessions: Vec::new(),
            observations: Vec::new(),
            prompts: Vec::new(),
            relations: Vec::new(),
        };

        let session_sql = match project {
            Some(_) => {
                "SELECT id, project, directory, started_at, ended_at, summary FROM sessions
                 WHERE project = ?1 OR id IN (
                    SELECT session_id FROM observations
                    WHERE ifnull(project, '') = ?1 OR
                          (ifnull(project, '') = '' AND session_id IN
                              (SELECT id FROM sessions WHERE project = ?1))
                    UNION
                    SELECT session_id FROM prompts
                    WHERE ifnull(project, '') = ?1 OR
                          (ifnull(project, '') = '' AND session_id IN
                              (SELECT id FROM sessions WHERE project = ?1))
                 ) ORDER BY datetime(started_at), id"
            }
            None => {
                "SELECT id, project, directory, started_at, ended_at, summary FROM sessions
                 ORDER BY datetime(started_at), id"
            }
        };
        let mut session_statement = self.connection.prepare(session_sql)?;
        let session_rows = match project {
            Some(project) => session_statement.query_map([project], map_session)?,
            None => session_statement.query_map([], map_session)?,
        };
        data.sessions = session_rows.collect::<Result<Vec<_>, _>>()?;

        let observation_sql = match project {
            Some(_) => format!(
                "SELECT {OBSERVATION_COLUMNS} FROM observations
                 WHERE ifnull(project, '') = ?1 OR
                       (ifnull(project, '') = '' AND session_id IN
                           (SELECT id FROM sessions WHERE project = ?1))
                 ORDER BY id"
            ),
            None => format!("SELECT {OBSERVATION_COLUMNS} FROM observations ORDER BY id"),
        };
        let mut observation_statement = self.connection.prepare(&observation_sql)?;
        let observation_rows = match project {
            Some(project) => observation_statement.query_map([project], map_observation)?,
            None => observation_statement.query_map([], map_observation)?,
        };
        data.observations = observation_rows.collect::<Result<Vec<_>, _>>()?;

        let prompt_sql = match project {
            Some(_) => {
                format!(
                    "SELECT {PROMPT_COLUMNS}
                 FROM prompts
                 WHERE ifnull(project, '') = ?1 OR
                       (ifnull(project, '') = '' AND session_id IN
                           (SELECT id FROM sessions WHERE project = ?1))
                 ORDER BY id"
                )
            }
            None => {
                format!(
                    "SELECT {PROMPT_COLUMNS}
                 FROM prompts ORDER BY id"
                )
            }
        };
        let mut prompt_statement = self.connection.prepare(&prompt_sql)?;
        let prompt_rows = match project {
            Some(project) => prompt_statement.query_map([project], map_prompt)?,
            None => prompt_statement.query_map([], map_prompt)?,
        };
        data.prompts = prompt_rows.collect::<Result<Vec<_>, _>>()?;
        drop(prompt_statement);

        // Only relations both of whose ends are in this export. One reaching a
        // memory that stayed behind would arrive at the far side as a claim
        // about something that is not there, and the import would have to throw
        // it away anyway.
        let relation_sql = format!(
            "SELECT {RELATION_COLUMNS} FROM memory_relations
             WHERE source_id IN (SELECT sync_id FROM observations WHERE {0})
               AND target_id IN (SELECT sync_id FROM observations WHERE {0})
             ORDER BY id",
            match project {
                Some(_) =>
                    "ifnull(project, '') = ?1 OR \
                            (ifnull(project, '') = '' AND session_id IN \
                                (SELECT id FROM sessions WHERE project = ?1))",
                None => "1 = 1",
            }
        );
        let mut relation_statement = self.connection.prepare(&relation_sql)?;
        let relation_rows = match project {
            Some(project) => relation_statement.query_map([project], map_relation)?,
            None => relation_statement.query_map([], map_relation)?,
        };
        data.relations = relation_rows.collect::<Result<Vec<_>, _>>()?;
        drop(relation_statement);
        // Nothing was written, so ending the snapshot either way is equivalent;
        // committing states the intent.
        snapshot.commit()?;
        Ok(data)
    }

    /// Enrolls a project for cloud replication. Only enrolled projects journal
    /// relation mutations, so this is the switch that opts data into sync.
    /// Enrols a project for cloud replication, and queues what it already holds.
    ///
    /// The queueing is the point. Nothing is journalled for a project nobody
    /// replicates — otherwise every store that never touches the cloud
    /// accumulates a full JSON copy of every write it ever made, for a reader
    /// that does not exist, and nothing removes them: pruning only reaches rows
    /// the cloud has acknowledged. One real store held 9 525 such rows against
    /// 3 408 memories, 14.5 MB of a 42 MB database.
    ///
    /// That trade is only safe because enrolling catches up. Without this, a
    /// project enrolled after it had been used would replicate whatever came
    /// next and silently never send its history.
    pub fn enroll_project(&mut self, project: &str) -> Result<bool, StoreError> {
        let project = normalize::project(project);
        if project.is_empty() {
            return Err(invalid_parameter(crate::project::EMPTY_NAME));
        }
        let tx = self.write_transaction()?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO sync_enrolled_projects (project) VALUES (?1)",
            [&project],
        )?;
        // Only on the first enrolment. Enrolling again is a no-op rather than a
        // second copy of everything.
        if inserted > 0 {
            // And whatever was queued for it before goes first.
            //
            // The backfill queues every row the project holds, in its current
            // state — that is the sentence that lets the journal skip an
            // unenrolled project without losing history, and it makes anything
            // already waiting for that project redundant by the same argument.
            // Left in place, a project enrolled a second time sends its stale
            // journal and then a full copy of itself.
            //
            // Unacked only. An acknowledged row is the record of what a peer
            // already has, and the retention window is what removes those.
            tx.execute(
                "DELETE FROM sync_mutations WHERE project = ?1 AND acked_at IS NULL",
                [&project],
            )?;
            backfill_project_tx(&tx, &project)?;
        }
        tx.commit()?;
        Ok(inserted > 0)
    }

    /// Removes a project from cloud replication. Already journaled mutations
    /// stay queued; this only stops new ones.
    pub fn unenroll_project(&mut self, project: &str) -> Result<bool, StoreError> {
        let project = normalize::project(project);
        let removed = self.connection.execute(
            "DELETE FROM sync_enrolled_projects WHERE project = ?1",
            [&project],
        )?;
        Ok(removed > 0)
    }

    pub fn enrolled_projects(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT project FROM sync_enrolled_projects ORDER BY project")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Reports whether a project already owns a session, observation, or prompt.
    /// MCP uses it to reject project names an agent invented.
    pub fn project_exists(&self, project: &str) -> Result<bool, StoreError> {
        let project = normalize::project(project);
        if project.is_empty() {
            return Ok(false);
        }
        Ok(self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sessions WHERE LOWER(TRIM(project)) = ?1
                 UNION ALL
                 SELECT 1 FROM observations
                 WHERE LOWER(TRIM(ifnull(project, ''))) = ?1 AND deleted_at IS NULL
                 UNION ALL
                 SELECT 1 FROM prompts WHERE LOWER(TRIM(ifnull(project, ''))) = ?1
             )",
            [&project],
            |row| row.get(0),
        )?)
    }

    pub fn list_project_names(&self) -> Result<Vec<String>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT project FROM observations
             WHERE project IS NOT NULL AND project != '' AND deleted_at IS NULL
             ORDER BY project",
        )?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Aggregates observation, session, and prompt counts per project, plus the
    /// distinct session directories each project has been used from.
    pub fn list_projects_with_stats(&self) -> Result<Vec<ProjectStats>, StoreError> {
        let mut projects: BTreeMap<String, ProjectStats> = BTreeMap::new();

        let mut observation_statement = self.connection.prepare(
            "SELECT project, COUNT(*) FROM observations
             WHERE project IS NOT NULL AND project != '' AND deleted_at IS NULL
             GROUP BY project",
        )?;
        let rows = observation_statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (name, count) = row?;
            projects
                .entry(name.clone())
                .or_insert_with(|| ProjectStats {
                    name,
                    ..ProjectStats::default()
                })
                .observation_count = count;
        }

        let mut session_statement = self.connection.prepare(
            "SELECT project, COUNT(*), ifnull(directory, '') FROM sessions
             WHERE project IS NOT NULL AND project != ''
             GROUP BY project, directory
             ORDER BY project, directory",
        )?;
        let rows = session_statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (name, count, directory) = row?;
            let stats = projects
                .entry(name.clone())
                .or_insert_with(|| ProjectStats {
                    name,
                    ..ProjectStats::default()
                });
            stats.session_count += count;
            if !directory.is_empty() && !stats.directories.contains(&directory) {
                stats.directories.push(directory);
            }
        }

        let mut prompt_statement = self.connection.prepare(
            "SELECT project, COUNT(*) FROM prompts
             WHERE project IS NOT NULL AND project != ''
             GROUP BY project",
        )?;
        let rows = prompt_statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (name, count) = row?;
            projects
                .entry(name.clone())
                .or_insert_with(|| ProjectStats {
                    name,
                    ..ProjectStats::default()
                })
                .prompt_count = count;
        }

        let mut projects = projects.into_values().collect::<Vec<_>>();
        projects.sort_by(|left, right| {
            right
                .observation_count
                .cmp(&left.observation_count)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(projects)
    }

    /// Removes the sessions and prompts of a project that holds no
    /// observations. Projects with observations are refused.
    pub fn prune_project(&mut self, project: &str) -> Result<PruneResult, StoreError> {
        let project = normalize::project(project);
        if project.is_empty() {
            return Err(invalid_parameter(crate::project::EMPTY_NAME));
        }
        // Soft-deleted rows count too. They still reference their sessions
        // through a NOT NULL foreign key, so deleting the sessions underneath
        // them would fail with a constraint error nobody could act on. Hard
        // delete the observations first, or use `delete_project --hard`.
        let observations = self.connection.query_row(
            "SELECT COUNT(*) FROM observations WHERE ifnull(project, '') = ?1",
            [&project],
            |row| row.get::<_, i64>(0),
        )?;
        if observations > 0 {
            return Err(invalid_parameter(format!(
                "project {project:?} still has {observations} observation(s), including \
                 soft-deleted ones that still reference its sessions"
            )));
        }

        let tx = self.write_transaction()?;
        let prompts = collect_prompts_tx(&tx, &project)?;
        let sessions = collect_sessions_tx(&tx, &project)?;
        let mut result = PruneResult {
            project: project.clone(),
            ..PruneResult::default()
        };
        for prompt in &prompts {
            enqueue_prompt_delete_tx(&tx, prompt)?;
        }
        result.prompts_deleted = tx.execute(
            "DELETE FROM prompts WHERE ifnull(project, '') = ?1",
            [&project],
        )? as i64;
        for session in &sessions {
            enqueue_session_delete_tx(&tx, session, &project)?;
        }
        result.sessions_deleted =
            tx.execute("DELETE FROM sessions WHERE project = ?1", [&project])? as i64;
        tx.commit()?;
        Ok(result)
    }

    /// Removes every observation, prompt, and session of a project. Soft
    /// deletion keeps the session rows because observations reference them.
    pub fn delete_project(
        &mut self,
        project: &str,
        hard_delete: bool,
    ) -> Result<DeleteProjectResult, StoreError> {
        let project = normalize::project(project);
        if project.is_empty() {
            return Err(invalid_parameter(crate::project::EMPTY_NAME));
        }
        let tx = self.write_transaction()?;
        let sessions = collect_sessions_tx(&tx, &project)?;
        let observation_ids = query_column(
            &tx,
            "SELECT id FROM observations WHERE ifnull(project, '') = ?1",
            &project,
        )?;
        if sessions.is_empty() && observation_ids.is_empty() {
            return Err(StoreError::ProjectNotFound(project));
        }
        let mut result = DeleteProjectResult {
            project: project.clone(),
            hard_delete,
            ..DeleteProjectResult::default()
        };
        let deleted_at = sqlite_now();

        for id in observation_ids {
            let observation = get_observation_row(&tx, id)?;
            if !hard_delete && observation.deleted_at.is_some() {
                continue;
            }
            if hard_delete {
                orphan_relations_tx(&tx, &observation.sync_id)?;
            }
            enqueue_mutation(
                &tx,
                "observation",
                &observation.sync_id,
                crate::sync::OP_DELETE,
                &serde_json::json!({
                    "sync_id": observation.sync_id,
                    "session_id": observation.session_id,
                    "project": observation.project,
                    "deleted": true,
                    "deleted_at": deleted_at,
                    "hard_delete": hard_delete,
                }),
                &project,
            )?;
            result.observations_deleted += 1;
        }
        if hard_delete {
            tx.execute(
                "DELETE FROM observations WHERE ifnull(project, '') = ?1",
                [&project],
            )?;
        } else {
            tx.execute(
                "UPDATE observations SET deleted_at = ?1, updated_at = datetime('now')
                 WHERE ifnull(project, '') = ?2 AND deleted_at IS NULL",
                params![deleted_at, project],
            )?;
        }

        for prompt in &collect_prompts_tx(&tx, &project)? {
            enqueue_prompt_delete_tx(&tx, prompt)?;
        }
        result.prompts_deleted = tx.execute(
            "DELETE FROM prompts WHERE ifnull(project, '') = ?1",
            [&project],
        )? as i64;

        if hard_delete {
            // Only the sessions that are now empty. A session belongs to one
            // project but the rows inside it carry their own, and an agent that
            // saved a prompt under a different name than the session it was
            // working in leaves that prompt behind here — deleting the session
            // out from under it broke the foreign key and failed the whole
            // delete, so a project with any such row could not be removed at
            // all. Those sessions stay, holding the rows that are not ours to
            // destroy.
            let emptied = query_column::<String>(
                &tx,
                "SELECT id FROM sessions s WHERE s.project = ?1
                   AND NOT EXISTS (SELECT 1 FROM observations o WHERE o.session_id = s.id)
                   AND NOT EXISTS (SELECT 1 FROM prompts p WHERE p.session_id = s.id)",
                &project,
            )?;
            for session in &emptied {
                enqueue_session_delete_tx(&tx, session, &project)?;
            }
            for session in &emptied {
                tx.execute("DELETE FROM sessions WHERE id = ?1", [session])?;
            }
            result.sessions_deleted = emptied.len() as i64;
            result.sessions_kept = sessions.len() as i64 - result.sessions_deleted;
        }

        tx.commit()?;
        Ok(result)
    }

    pub fn merge_project(
        &mut self,
        source: &str,
        canonical: &str,
    ) -> Result<MergeResult, StoreError> {
        self.merge_projects(&[source.to_owned()], canonical)
    }

    pub fn merge_projects(
        &mut self,
        sources: &[String],
        canonical: &str,
    ) -> Result<MergeResult, StoreError> {
        let canonical = normalize::project(canonical);
        if canonical.is_empty() {
            return Err(invalid_parameter(crate::project::EMPTY_NAME));
        }
        let tx = self.write_transaction()?;
        // Preguntado antes de mover nada, que es lo único que lo hace medible:
        // en cuanto la primera fila cambia de nombre, el destino existe.
        let canonical_existed: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sessions WHERE LOWER(TRIM(project)) = ?1
                 UNION ALL
                 SELECT 1 FROM observations
                 WHERE LOWER(TRIM(ifnull(project, ''))) = ?1 AND deleted_at IS NULL
                 UNION ALL
                 SELECT 1 FROM prompts WHERE LOWER(TRIM(ifnull(project, ''))) = ?1
             )",
            [&canonical],
            |row| row.get(0),
        )?;
        let mut result = MergeResult {
            canonical: canonical.clone(),
            ..MergeResult::default()
        };
        let mut seen = BTreeSet::new();

        for source_input in sources {
            let source = normalize::project(source_input);
            if source.is_empty() || source == canonical || !seen.insert(source.clone()) {
                continue;
            }
            let variants = project_merge_variants(source_input, &source, &canonical);
            let mut observation_ids = BTreeSet::new();
            let mut session_ids = BTreeSet::new();
            let mut prompt_ids = BTreeSet::new();
            for variant in &variants {
                observation_ids.extend(query_column::<i64>(
                    &tx,
                    "SELECT id FROM observations
                     WHERE LOWER(TRIM(ifnull(project, ''))) = ?1",
                    variant,
                )?);
                session_ids.extend(query_column::<String>(
                    &tx,
                    "SELECT id FROM sessions WHERE LOWER(TRIM(project)) = ?1",
                    variant,
                )?);
                prompt_ids.extend(query_column::<i64>(
                    &tx,
                    "SELECT id FROM prompts
                     WHERE LOWER(TRIM(ifnull(project, ''))) = ?1",
                    variant,
                )?);
            }
            if observation_ids.is_empty() && session_ids.is_empty() && prompt_ids.is_empty() {
                continue;
            }
            // Replication follows the memories, and it has to be arranged
            // before they are queued below rather than after.
            //
            // Nothing is journalled for a project nobody replicates, so a
            // merge into an unenrolled name queued every moved row into
            // nowhere: the memories arrived under the new name, the peers were
            // never told, and `cloud status` went on listing the old project,
            // which by then held nothing at all. Replication stopped and
            // nothing said so.
            //
            // Enrolling the canonical here rather than at the end is the whole
            // of it: `enqueue_observation` and the two `enqueue_mutation`
            // calls under this block ask `is_enrolled_tx` and return quietly
            // when the answer is no.
            if is_enrolled_tx(&tx, &source)? && !is_enrolled_tx(&tx, &canonical)? {
                tx.execute(
                    "INSERT OR IGNORE INTO sync_enrolled_projects (project) VALUES (?1)",
                    [&canonical],
                )?;
                result.enrolment_moved = true;
            }

            for variant in &variants {
                result.observations_updated += tx.execute(
                    "UPDATE observations SET project = ?1, updated_at = datetime('now')
                     WHERE LOWER(TRIM(ifnull(project, ''))) = ?2",
                    params![canonical, variant],
                )? as i64;
                result.sessions_updated += tx.execute(
                    "UPDATE sessions SET project = ?1 WHERE LOWER(TRIM(project)) = ?2",
                    params![canonical, variant],
                )? as i64;
                result.prompts_updated += tx.execute(
                    "UPDATE prompts SET project = ?1
                     WHERE LOWER(TRIM(ifnull(project, ''))) = ?2",
                    params![canonical, variant],
                )? as i64;
            }

            for id in observation_ids {
                let observation = get_observation_row(&tx, id)?;
                if observation.deleted_at.is_none() {
                    enqueue_observation(&tx, &observation)?;
                }
            }
            for id in session_ids {
                let session = get_session_row(&tx, &id)?;
                enqueue_mutation(
                    &tx,
                    "session",
                    &id,
                    crate::sync::OP_UPSERT,
                    &session,
                    &canonical,
                )?;
            }
            for id in prompt_ids {
                let prompt = get_prompt_tx(&tx, id)?;
                enqueue_mutation(
                    &tx,
                    "prompt",
                    &prompt.sync_id,
                    crate::sync::OP_UPSERT,
                    &prompt,
                    &canonical,
                )?;
            }
            // And the name that is now empty stops being replicated. Its
            // unsent mutations go with it: every row they described has just
            // been queued again, in its current state, under the canonical
            // name — which is the same argument that lets an unenrolled
            // project skip the journal without losing history.
            tx.execute(
                "DELETE FROM sync_enrolled_projects WHERE project = ?1",
                [&source],
            )?;
            tx.execute(
                "DELETE FROM sync_mutations WHERE project = ?1 AND acked_at IS NULL",
                [&source],
            )?;
            result.sources_merged.push(source);
        }

        // What the merge left sharing a name.
        //
        // Counted after every source has moved rather than once per source: two
        // sources can each bring a memory under the same key, and the collision
        // only exists once both are in. See `MergeResult::topic_key_collisions`.
        result.topic_key_collisions = tx.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT topic_key FROM observations
                  WHERE deleted_at IS NULL AND topic_key IS NOT NULL
                    AND ifnull(project, '') = ?1
                  GROUP BY topic_key, scope HAVING COUNT(*) > 1)",
            [&canonical],
            |row| row.get(0),
        )?;

        // Only when something actually moved: asking to merge a name nothing
        // holds into another name nothing holds changes nothing, and saying a
        // project was created there would be inventing an event.
        result.canonical_created = !canonical_existed && !result.sources_merged.is_empty();
        tx.commit()?;
        Ok(result)
    }
}
