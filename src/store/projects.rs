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
            observation_versions: Vec::new(),
            // Leteo writes `0.1.0`, which has no tombstones. The field exists
            // to read Engram 0.2.0's; producing them is not this build's job.
            prompt_tombstones: Vec::new(),
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
        // The version history of the memories this export holds. One whose
        // memory stayed behind is left out with it: a version is only ever read
        // through the memory it belongs to, so shipping one alone would be a
        // row nothing can reach.
        let version_sql = format!(
            "SELECT observation_sync_id, revision, title, content, replaced_at
               FROM observation_versions
              WHERE observation_sync_id IN (SELECT sync_id FROM observations WHERE {0})
              ORDER BY observation_sync_id, revision DESC",
            match project {
                Some(_) =>
                    "ifnull(project, '') = ?1 OR \
                            (ifnull(project, '') = '' AND session_id IN \
                                (SELECT id FROM sessions WHERE project = ?1))",
                None => "1 = 1",
            }
        );
        let mut version_statement = self.connection.prepare(&version_sql)?;
        let version_rows = match project {
            Some(project) => {
                version_statement.query_map([project], map_export_observation_version)?
            }
            None => version_statement.query_map([], map_export_observation_version)?,
        };
        data.observation_versions = version_rows.collect::<Result<Vec<_>, _>>()?;
        drop(version_statement);
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
            //
            // And never a delete. The backfill queues what the project holds
            // now, so it says nothing about a memory, prompt or session that is
            // gone, and the unsent delete was the only copy of that fact:
            // dropping it left the peer with a row its owner had removed.
            tx.execute(
                "DELETE FROM sync_mutations
                  WHERE project = ?1 AND acked_at IS NULL AND op != ?2",
                params![project, crate::sync::OP_DELETE],
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

    /// Reports whether a project already owns an observation, a prompt, or a
    /// session a caller declared with `mem_session_start`.
    /// MCP uses it to reject project names an agent invented.
    ///
    /// An empty session a hook opened does not count: a hook opens one wherever
    /// an agent starts, so counting it made every directory's name a project an
    /// agent could then write to.
    pub fn project_exists(&self, project: &str) -> Result<bool, StoreError> {
        let project = normalize::project(project);
        if project.is_empty() {
            return Ok(false);
        }
        Ok(self.connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sessions
                 WHERE LOWER(TRIM(project)) = ?1 AND declared = 1
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

    /// Aggregates observation, session, and prompt counts per project, the
    /// newest instant anything happened in it, and the distinct session
    /// directories each project has been used from.
    pub fn list_projects_with_stats(&self) -> Result<Vec<ProjectStats>, StoreError> {
        let mut projects: BTreeMap<String, ProjectStats> = BTreeMap::new();

        let mut observation_statement = self.connection.prepare(
            "SELECT project, COUNT(*), MAX(datetime(created_at)) FROM observations
             WHERE project IS NOT NULL AND project != '' AND deleted_at IS NULL
             GROUP BY project",
        )?;
        let rows = observation_statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (name, count, at) = row?;
            let stats = projects
                .entry(name.clone())
                .or_insert_with(|| ProjectStats {
                    name,
                    ..ProjectStats::default()
                });
            stats.observation_count = count;
            keep_later(&mut stats.last_activity, at);
        }

        let mut session_statement = self.connection.prepare(
            "SELECT project, COUNT(*), ifnull(directory, ''),
                    MAX(datetime(COALESCE(ended_at, started_at))) FROM sessions
             WHERE project IS NOT NULL AND project != ''
             GROUP BY project, directory
             ORDER BY project, directory",
        )?;
        let rows = session_statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        for row in rows {
            let (name, count, directory, at) = row?;
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
            keep_later(&mut stats.last_activity, at);
        }

        let mut prompt_statement = self.connection.prepare(
            "SELECT project, COUNT(*), MAX(datetime(created_at)) FROM prompts
             WHERE project IS NOT NULL AND project != ''
             GROUP BY project",
        )?;
        let rows = prompt_statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (name, count, at) = row?;
            let stats = projects
                .entry(name.clone())
                .or_insert_with(|| ProjectStats {
                    name,
                    ..ProjectStats::default()
                });
            stats.prompt_count = count;
            keep_later(&mut stats.last_activity, at);
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

    /// The per-project counts an agent reads through `mem_stats`.
    ///
    /// `names` is the list to answer for, already in the order the reply sends
    /// them: `mem_stats` reads it from [`Store::stats`], which resolves each
    /// project by seek rather than by grouping the whole table — the seek
    /// rather than the grouping. Each name in the head is then one query,
    /// so the work is proportional to the ceiling and not to the store, and the
    /// names past the ceiling are counted rather than answered.
    ///
    /// The `directories` [`ProjectStats`] also carries are left empty: `mem_stats`
    /// sends no directory, and filling them here would be a second query per
    /// project for a field nobody reads.
    pub fn project_stats_for(
        &self,
        names: &[String],
        limit: usize,
    ) -> Result<(Vec<ProjectStats>, usize), StoreError> {
        let omitted = names.len().saturating_sub(limit);
        let mut statement = self.connection.prepare(PROJECT_STAT_SQL)?;
        let mut projects = Vec::with_capacity(limit.min(names.len()));
        for name in names.iter().take(limit) {
            let (observation_count, session_count, prompt_count, last_activity) = statement
                .query_row([name], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })?;
            projects.push(ProjectStats {
                name: name.clone(),
                observation_count,
                session_count,
                prompt_count,
                directories: Vec::new(),
                last_activity,
            });
        }
        Ok((projects, omitted))
    }

    /// Why [`Self::prune_project`] would refuse `project`, or `None` when it
    /// would not.
    ///
    /// Soft-deleted rows count. They still reference their sessions through a
    /// NOT NULL foreign key, so deleting the sessions underneath them would fail
    /// with a constraint error nobody could act on. Hard delete the observations
    /// first, or use `delete_project --hard`.
    ///
    /// The one place the rule lives: `projects prune` lists its candidates by
    /// live count, which says nothing about soft-deleted rows, and asks here
    /// before it applies so that one such project does not abort the others.
    pub fn prune_blocker(&self, project: &str) -> Result<Option<String>, StoreError> {
        let project = normalize::project(project);
        if project.is_empty() {
            return Err(invalid_parameter(crate::project::EMPTY_NAME));
        }
        let observations = self.connection.query_row(
            "SELECT COUNT(*) FROM observations WHERE ifnull(project, '') = ?1",
            [&project],
            |row| row.get::<_, i64>(0),
        )?;
        Ok((observations > 0).then(|| {
            format!(
                "project {project:?} still has {observations} observation(s), including \
                 soft-deleted ones that still reference its sessions"
            )
        }))
    }

    /// Removes the sessions and prompts of a project that holds no
    /// observations. Projects with observations are refused.
    pub fn prune_project(&mut self, project: &str) -> Result<PruneResult, StoreError> {
        let project = normalize::project(project);
        if let Some(reason) = self.prune_blocker(&project)? {
            return Err(invalid_parameter(reason));
        }

        let tx = self.write_transaction()?;
        let prompts = collect_prompts_tx(&tx, &project)?;
        let mut result = PruneResult {
            project: project.clone(),
            ..PruneResult::default()
        };
        for prompt in &prompts {
            prompts::erase_deleted_prompt_tx(
                &tx,
                &prompt.sync_id,
                &prompt.session_id,
                &prompt.project,
                None,
            )?;
            enqueue_prompt_delete_tx(&tx, prompt)?;
        }
        result.prompts_deleted = tx.execute(
            "DELETE FROM prompts WHERE ifnull(project, '') = ?1",
            [&project],
        )? as i64;
        // After the prompts above are gone, so what still references a session
        // is another project's. Deleting those sessions regardless failed the
        // whole prune with a bare foreign-key error, for a project whose only
        // fault was having been the name on a session that another project's
        // memory was saved into.
        let emptied = sessions_holding_nothing_tx(&tx, &project)?;
        for session in &emptied {
            enqueue_session_delete_tx(&tx, session, &project)?;
        }
        for session in &emptied {
            tx.execute("DELETE FROM sessions WHERE id = ?1", [session])?;
        }
        result.sessions_deleted = emptied.len() as i64;
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
                observations::erase_hard_deleted_observation_tx(&tx, &observation.sync_id)?;
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
            prompts::erase_deleted_prompt_tx(
                &tx,
                &prompt.sync_id,
                &prompt.session_id,
                &prompt.project,
                None,
            )?;
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
            let emptied = sessions_holding_nothing_tx(&tx, &project)?;
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

    /// Moves the sessions of `source` that were recorded from `directory`, with
    /// the memories and prompts filed under `source` by those sessions, and
    /// nothing else.
    ///
    /// What a hook may do on its own. [`Self::merge_project`] moves every row
    /// under a name, wherever it was recorded, and also the spellings joined by
    /// space, hyphen and underscore that are not projects of their own; a
    /// project called `api` recorded from two checkouts lost the other one's
    /// memories to whichever opened a session first, with no prompt and no
    /// report. Neither the rest of the name nor its variants are touched here.
    ///
    /// Returns the result of the move and how many sessions under `source` were
    /// recorded elsewhere and stayed. The directory comparison is
    /// [`same_directory`](crate::project::same_directory)'s, in Rust, for the
    /// reason `session_directories` gives.
    ///
    /// Replication follows the rows exactly as `merge_projects` arranges it:
    /// the canonical project takes over the enrolment, every moved row is
    /// queued again under the new name, and the source's enrolment goes, and its
    /// unsent mutations move to the new name, only when the move left it empty.
    /// Touching them while sessions stay behind would silence a project that
    /// still holds rows.
    ///
    /// No rename is queued: it moves every row of a name on the peer, and this
    /// moves the rows of one directory. A peer that has not applied the moved
    /// rows' upserts still files them under `source`.
    pub fn move_directory_sessions(
        &mut self,
        source: &str,
        canonical: &str,
        directory: &Path,
    ) -> Result<(MergeResult, i64), StoreError> {
        let source = normalize::project(source);
        let canonical = normalize::project(canonical);
        if canonical.is_empty() {
            return Err(invalid_parameter(crate::project::EMPTY_NAME));
        }
        let mut result = MergeResult {
            canonical: canonical.clone(),
            ..MergeResult::default()
        };
        if source.is_empty() || source == canonical {
            return Ok((result, 0));
        }

        let tx = self.write_transaction()?;
        let canonical_existed: bool =
            tx.query_row(PROJECT_LIVE_ROWS_SQL, [&canonical], |row| row.get(0))?;

        let mut session_ids = Vec::new();
        let mut left_behind = 0_i64;
        {
            let mut statement =
                tx.prepare("SELECT id, directory FROM sessions WHERE LOWER(TRIM(project)) = ?1")?;
            let rows = statement.query_map([&source], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            for row in rows {
                let (id, recorded) = row?;
                if recorded
                    .as_deref()
                    .is_some_and(|recorded| crate::project::same_directory(recorded, directory))
                {
                    session_ids.push(id);
                } else {
                    left_behind += 1;
                }
            }
        }
        if session_ids.is_empty() {
            return Ok((result, left_behind));
        }

        let mut observation_ids = Vec::new();
        let mut prompt_ids = Vec::new();
        {
            let mut observations = tx.prepare(
                "SELECT id FROM observations
                 WHERE session_id = ?1 AND LOWER(TRIM(ifnull(project, ''))) = ?2",
            )?;
            let mut prompts = tx.prepare(
                "SELECT id FROM prompts
                 WHERE session_id = ?1 AND LOWER(TRIM(ifnull(project, ''))) = ?2",
            )?;
            for session in &session_ids {
                for id in
                    observations.query_map(params![session, source], |row| row.get::<_, i64>(0))?
                {
                    observation_ids.push(id?);
                }
                for id in prompts.query_map(params![session, source], |row| row.get::<_, i64>(0))? {
                    prompt_ids.push(id?);
                }
            }
        }

        if is_enrolled_tx(&tx, &source)? && !is_enrolled_tx(&tx, &canonical)? {
            tx.execute(
                "INSERT OR IGNORE INTO sync_enrolled_projects (project) VALUES (?1)",
                [&canonical],
            )?;
            result.enrolment_moved = true;
        }

        for id in &observation_ids {
            result.observations_updated += tx.execute(
                "UPDATE observations SET project = ?1, updated_at = datetime('now')
                 WHERE id = ?2",
                params![canonical, id],
            )? as i64;
        }
        for id in &prompt_ids {
            result.prompts_updated += tx.execute(
                "UPDATE prompts SET project = ?1 WHERE id = ?2",
                params![canonical, id],
            )? as i64;
        }
        for id in &session_ids {
            result.sessions_updated += tx.execute(
                "UPDATE sessions SET project = ?1 WHERE id = ?2",
                params![canonical, id],
            )? as i64;
        }

        for id in observation_ids {
            let observation = get_observation_row(&tx, id)?;
            if observation.deleted_at.is_none() {
                enqueue_observation(&tx, &observation)?;
                enqueue_stored_versions_tx(&tx, &observation.sync_id, &canonical)?;
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

        let source_is_empty: bool = tx.query_row(
            "SELECT NOT EXISTS(
                 SELECT 1 FROM sessions WHERE LOWER(TRIM(project)) = ?1
                 UNION ALL
                 SELECT 1 FROM observations WHERE LOWER(TRIM(ifnull(project, ''))) = ?1
                 UNION ALL
                 SELECT 1 FROM prompts WHERE LOWER(TRIM(ifnull(project, ''))) = ?1
             )",
            [&source],
            |row| row.get(0),
        )?;
        if source_is_empty {
            tx.execute(
                "DELETE FROM sync_enrolled_projects WHERE project = ?1",
                [&source],
            )?;
            carry_unsent_mutations_tx(&tx, &source, &canonical)?;
        }
        result.sources_merged.push(source);

        result.topic_key_collisions = tx.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT topic_key FROM observations
                  WHERE deleted_at IS NULL AND topic_key IS NOT NULL
                    AND ifnull(project, '') = ?1
                  GROUP BY topic_key, scope HAVING COUNT(*) > 1)",
            [&canonical],
            |row| row.get(0),
        )?;
        result.canonical_created = !canonical_existed;
        tx.commit()?;
        Ok((result, left_behind))
    }

    /// The names a merge of `sources` into `canonical` would move.
    ///
    /// Read-only, and the same read [`Self::merge_projects`] performs inside
    /// its transaction, so `projects consolidate`'s preview cannot promise a
    /// different set from the one the answer names.
    pub fn project_merge_scope(
        &self,
        sources: &[String],
        canonical: &str,
    ) -> Result<Vec<String>, StoreError> {
        let canonical = normalize::project(canonical);
        if canonical.is_empty() {
            return Err(invalid_parameter(crate::project::EMPTY_NAME));
        }
        Ok(merge_scope(&self.connection, sources, &canonical)?
            .into_iter()
            .collect())
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
        let canonical_existed: bool =
            tx.query_row(PROJECT_LIVE_ROWS_SQL, [&canonical], |row| row.get(0))?;
        let mut result = MergeResult {
            canonical: canonical.clone(),
            ..MergeResult::default()
        };

        let names = merge_scope(&tx, sources, &canonical)?;
        if !names.is_empty() {
            let mut observation_ids = BTreeSet::new();
            let mut session_ids = BTreeSet::new();
            let mut prompt_ids = BTreeSet::new();
            for name in &names {
                observation_ids.extend(query_column::<i64>(
                    &tx,
                    "SELECT id FROM observations
                     WHERE LOWER(TRIM(ifnull(project, ''))) = ?1",
                    name,
                )?);
                session_ids.extend(query_column::<String>(
                    &tx,
                    "SELECT id FROM sessions WHERE LOWER(TRIM(project)) = ?1",
                    name,
                )?);
                prompt_ids.extend(query_column::<i64>(
                    &tx,
                    "SELECT id FROM prompts
                     WHERE LOWER(TRIM(ifnull(project, ''))) = ?1",
                    name,
                )?);
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
            if move_enrolment_tx(&tx, &names, &canonical)? {
                result.enrolment_moved = true;
            }

            let (observations, sessions, prompts) =
                reassign_project_rows_tx(&tx, &names, &canonical)?;
            result.observations_updated += observations;
            result.sessions_updated += sessions;
            result.prompts_updated += prompts;

            // Ahead of every row queued again below, and of the unsent
            // mutations carried after them: a peer applies the queue in order,
            // and a mutation filed under the new name finds its row only once
            // the peer has moved it.
            enqueue_project_rename_tx(&tx, &names, &canonical)?;

            for id in observation_ids {
                let observation = get_observation_row(&tx, id)?;
                if observation.deleted_at.is_none() {
                    enqueue_observation(&tx, &observation)?;
                    enqueue_stored_versions_tx(&tx, &observation.sync_id, &canonical)?;
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
            // And the names that are now empty stop being replicated. Their
            // unsent mutations do not go with them: see
            // `carry_unsent_mutations_tx` for what is dropped and what moves to
            // the canonical name.
            retire_project_names_tx(&tx, &names, &canonical)?;
            result.sources_merged = names.into_iter().collect();
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

/// Whether any row of a person's is filed under `?1`: a session, a prompt, or
/// a memory that is not soft-deleted.
///
/// One statement, because the merge's "did the canonical exist" question, the
/// directory move's and the liveness test below all have to ask it the same
/// way. `project_exists` is deliberately not one of them: it counts only the
/// sessions a caller declared, and an empty hook session must not turn a
/// directory's name into a project an agent can write to.
const PROJECT_LIVE_ROWS_SQL: &str = "SELECT EXISTS(
     SELECT 1 FROM sessions WHERE LOWER(TRIM(project)) = ?1
     UNION ALL
     SELECT 1 FROM observations
     WHERE LOWER(TRIM(ifnull(project, ''))) = ?1 AND deleted_at IS NULL
     UNION ALL
     SELECT 1 FROM prompts WHERE LOWER(TRIM(ifnull(project, ''))) = ?1
 )";

/// The same, without the deletion filter.
///
/// A name holding only soft-deleted memories is not a live project, so a merge
/// folds it like any other spelling — but it still holds rows that have to move
/// with the name, and this is what says the source contributes something.
const PROJECT_ROWS_SQL: &str = "SELECT EXISTS(
     SELECT 1 FROM sessions WHERE LOWER(TRIM(project)) = ?1
     UNION ALL
     SELECT 1 FROM observations
     WHERE LOWER(TRIM(ifnull(project, ''))) = ?1
     UNION ALL
     SELECT 1 FROM prompts WHERE LOWER(TRIM(ifnull(project, ''))) = ?1
 )";

fn project_holds_any_row(connection: &Connection, project: &str) -> Result<bool, StoreError> {
    Ok(connection.query_row(PROJECT_ROWS_SQL, [project], |row| row.get(0))?)
}

/// Whether a name is a project in its own right rather than a spelling of the
/// one being merged.
///
/// Enrolment or a row of its own is what makes it one. `my-app` and `my_app`
/// are separate projects everywhere else — listing, deleting, existence, all
/// read the name literally — and joining them took an unrelated project's
/// memories and stopped it replicating, while the reply named only the project
/// the caller typed.
fn project_is_live(connection: &Connection, project: &str) -> Result<bool, StoreError> {
    if is_enrolled(connection, project)? {
        return Ok(true);
    }
    Ok(connection.query_row(PROJECT_LIVE_ROWS_SQL, [project], |row| row.get(0))?)
}

/// The names folding one source would move: the spelling the caller typed, its
/// normalized form, and every separator variant that is not a live project.
fn merge_source_names(
    connection: &Connection,
    source_input: &str,
    source: &str,
    canonical: &str,
) -> Result<BTreeSet<String>, StoreError> {
    let typed = source_input.trim().to_lowercase();
    let mut candidates = project_merge_variants(source_input, source, canonical);
    candidates.insert(source.to_owned());
    let mut names = BTreeSet::new();
    for name in candidates {
        // The spelling the caller typed is the name they asked to merge, and it
        // moves however alive it is. Only a spelling they did not name has to
        // prove it is not a project of its own.
        if name == source || name == typed || !project_is_live(connection, &name)? {
            names.insert(name);
        }
    }
    Ok(names)
}

/// Every name a merge of `sources` into `canonical` moves.
///
/// The one read that decides it, shared by [`Store::merge_projects`] and
/// [`Store::project_merge_scope`] so the answer and the preview cannot drift.
fn merge_scope(
    connection: &Connection,
    sources: &[String],
    canonical: &str,
) -> Result<BTreeSet<String>, StoreError> {
    let mut seen = BTreeSet::new();
    let mut names = BTreeSet::new();
    for source_input in sources {
        let source = normalize::project(source_input);
        if source.is_empty() || source == canonical || !seen.insert(source.clone()) {
            continue;
        }
        let source_names = merge_source_names(connection, source_input, &source, canonical)?;
        // Only a name with a row of its own is moved. A name nobody filed
        // anything under is a spelling no store has ever held: moving it would
        // invent an event, and reporting it would name a project that never
        // existed. A source the caller enrolled but never wrote to has nothing
        // to move either, which is what the merge did before it had a scope to
        // report. The caller's own spelling is added once the source moves
        // something, because that is the name they asked to merge.
        let mut moved = BTreeSet::new();
        for name in &source_names {
            if project_holds_any_row(connection, name)? {
                moved.insert(name.clone());
            }
        }
        if !moved.is_empty() {
            moved.insert(source.clone());
            names.extend(moved);
        }
    }
    Ok(names)
}

/// One project's counts and last activity, answered by seek rather than by
/// grouping the whole table. `?1` is the project name; the three counts and the
/// newest instant are indexed lookups, which is what `mem_stats` pays per
/// project it lists and why the list's own ceiling bounds the work.
///
/// The newest instant unions the project's live memories, its sessions (the end
/// when there is one, the start otherwise) and its prompts, then takes the
/// maximum. `datetime(...)` normalises each to the one `YYYY-MM-DD HH:MM:SS`
/// shape that compares correctly as a string.
pub(crate) const PROJECT_STAT_SQL: &str = "SELECT
       (SELECT COUNT(*) FROM observations
         WHERE project = ?1 AND deleted_at IS NULL),
       (SELECT COUNT(*) FROM sessions WHERE project = ?1),
       (SELECT COUNT(*) FROM prompts WHERE project = ?1),
       (SELECT MAX(at) FROM (
            SELECT datetime(created_at) AS at FROM observations
             WHERE project = ?1 AND deleted_at IS NULL
            UNION ALL
            SELECT datetime(COALESCE(ended_at, started_at)) FROM sessions
             WHERE project = ?1
            UNION ALL
            SELECT datetime(created_at) FROM prompts WHERE project = ?1))";

/// Keeps the later of two SQLite `datetime(...)` readings.
///
/// They sort as strings, which is why every ordering in this codebase reads
/// them through `datetime(...)` in the first place: one fixed
/// `YYYY-MM-DD HH:MM:SS` shape compares correctly with `>`. `None` means the
/// group carried no timestamp at all, and never displaces a real one.
fn keep_later(current: &mut Option<String>, candidate: Option<String>) {
    let Some(candidate) = candidate else {
        return;
    };
    let later = match current.as_deref() {
        Some(held) => candidate.as_str() > held,
        None => true,
    };
    if later {
        *current = Some(candidate);
    }
}
