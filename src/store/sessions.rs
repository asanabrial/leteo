//! Sessions: the unit of work a memory was written during.

use super::*;

impl Store {
    /// The directories any session of this project was opened in.
    ///
    /// For the one question the session-start hook asks on every run: has this
    /// directory been seen under another project name, i.e. was the project
    /// renamed? It used to be answered with `list_projects_with_stats`, which
    /// aggregates **every** project in the store and then keeps one — 6 ms of
    /// `GROUP BY` over 3,550 memories to look at a handful of rows, paid every
    /// time a session opens.
    ///
    /// The comparison itself stays in Rust: the same directory is written in
    /// several ways across machines, and the directory comparison folds separators and
    /// case in a way SQL here should not try to copy.
    pub fn session_directories(&self, project: &str) -> Result<Vec<String>, StoreError> {
        let project = normalize::project(project);
        let mut statement = self
            .connection
            .prepare("SELECT DISTINCT directory FROM sessions WHERE project = ?1")?;
        let rows = statement.query_map(params![project], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The projects this directory's sessions were recorded under, when it does
    /// not resolve to the same one now.
    ///
    /// The directory's own bucket clears the question; a session that merely
    /// visited does not. One stray row — what the session door used to write
    /// when it opened a session here for another project — used to make the
    /// whole directory disputed for ever (#405 stopped new ones, not the
    /// stores holding one). But any row naming the detected project is too
    /// wide: a hook records the directory before anybody has said it is
    /// theirs, so after a rename it would answer the question the rename
    /// exists to ask.
    ///
    /// The MCP door refused the silent pick and `leteo save` filed it under the
    /// name detection now reads, so the same directory accepted a memory
    /// through one door and rejected it through the other. Two questions let
    /// the two answers drift apart; there is one, and both doors ask it.
    ///
    /// `is_manual` is the caller's because the id vocabulary lives with the
    /// door that mints it; the store does not learn a prefix here.
    pub fn drifted_projects(
        &self,
        detection: &crate::project::ProjectDetection,
        is_manual: impl Fn(&str) -> bool,
    ) -> Result<Vec<String>, StoreError> {
        let detected = normalize::project(&detection.project);
        if detected.is_empty() {
            return Ok(Vec::new());
        }
        if self.directory_has_manual_session(&detection.path, &detected, is_manual)? {
            return Ok(Vec::new());
        }
        self.recent_projects_in_directory(&detection.path, &detected)
    }

    /// The projects of sessions recorded in exactly this directory, most recent
    /// first, excluding `exclude`.
    ///
    /// What a session-start hook and an MCP write ask before believing the
    /// project detection derived from `origin` this time: a repository that
    /// gained a remote, renamed it, or was pointed at a fork resolves to a new
    /// name, and the sessions already recorded in this directory are the
    /// evidence that it used to be another. Without the lookup the new name is
    /// picked silently and the earlier memories stay under the old one.
    ///
    /// The directory comparison is in Rust rather than in SQL for the reason
    /// [`DirectoryTarget`](crate::project::DirectoryTarget) gives: the same
    /// directory is written with a trailing separator, with backslashes, and on
    /// Windows with a different case, and SQLite's `LOWER` cannot fold any of
    /// that the way the filesystem means it. So the row filter is only "has a
    /// directory and a project at all"; the narrowing to this directory happens
    /// here. The scan is over `sessions` alone, and a session row is one line,
    /// not a body.
    pub fn recent_projects_in_directory(
        &self,
        directory: &str,
        exclude: &str,
    ) -> Result<Vec<String>, StoreError> {
        let exclude = normalize::project(exclude);
        let mut statement = self.connection.prepare(
            "SELECT project, directory, COALESCE(started_at, '')
             FROM sessions
             WHERE directory IS NOT NULL AND trim(directory) <> ''
               AND project IS NOT NULL AND trim(project) <> ''
             ORDER BY COALESCE(started_at, '') DESC, id DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut target = crate::project::DirectoryTarget::new(Path::new(directory));
        let mut projects = Vec::new();
        for row in rows {
            let (project, recorded) = row?;
            let project = normalize::project(&project);
            if project.is_empty() || project == exclude {
                continue;
            }
            if !target.matches(&recorded) {
                continue;
            }
            if !projects.contains(&project) {
                projects.push(project);
            }
        }
        Ok(projects)
    }

    /// Whether `directory` holds the manual session a silent save joins under
    /// `project` — the directory used under that name, not merely visited.
    ///
    /// The predicate is the caller's because the id vocabulary lives with the
    /// door that mints it; the store does not learn a prefix here.
    pub fn directory_has_manual_session(
        &self,
        directory: &str,
        project: &str,
        is_manual: impl Fn(&str) -> bool,
    ) -> Result<bool, StoreError> {
        let project = normalize::project(project);
        if project.is_empty() {
            return Ok(false);
        }
        let mut statement = self.connection.prepare(
            "SELECT id, project, directory FROM sessions
             WHERE directory IS NOT NULL AND trim(directory) <> ''",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut target = crate::project::DirectoryTarget::new(Path::new(directory));
        for row in rows {
            let (id, recorded_project, recorded) = row?;
            if !is_manual(&id) || normalize::project(&recorded_project) != project {
                continue;
            }
            if target.matches(&recorded) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Takes `directory` away from every session recorded in it under a project
    /// other than `keep`, and returns how many sessions that was.
    ///
    /// The other half of [`Self::recent_projects_in_directory`]: that reads the
    /// directory's sessions as evidence it used to be called something else,
    /// and this is what a person's answer that it never was does to the
    /// evidence. The sessions stay under their own project with everything they
    /// hold; only the claim that they were opened here goes, which is the one
    /// thing the answer contradicts. The directory is matched in Rust for the
    /// reason given there.
    ///
    /// Each changed session is queued for a peer, because a peer that kept the
    /// directory would go on asking the question this store has stopped asking.
    pub fn release_directory_to(
        &mut self,
        directory: &str,
        keep: &str,
    ) -> Result<usize, StoreError> {
        let keep = normalize::project(keep);
        let tx = self.write_transaction()?;
        let mut target = crate::project::DirectoryTarget::new(Path::new(directory));
        let mut released = Vec::new();
        {
            let mut statement = tx.prepare(
                "SELECT id, project, directory FROM sessions
                 WHERE directory IS NOT NULL AND trim(directory) <> ''",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (id, project, recorded) = row?;
                if normalize::project(&project) != keep && target.matches(&recorded) {
                    released.push(id);
                }
            }
        }
        for id in &released {
            tx.execute("UPDATE sessions SET directory = '' WHERE id = ?1", [id])?;
            let session = get_session_row(&tx, id)?;
            enqueue_mutation(
                &tx,
                "session",
                id,
                crate::sync::OP_UPSERT,
                &session,
                &session.project,
            )?;
        }
        tx.commit()?;
        Ok(released.len())
    }

    pub fn create_session(
        &mut self,
        id: &str,
        project: &str,
        directory: &str,
    ) -> Result<Session, StoreError> {
        self.create_session_marked(id, project, directory, false)
    }

    /// Creates a session a caller opened to bring its project into being, which
    /// is what `project_exists` counts even while the session is empty.
    ///
    /// The flag is only ever raised: a session a hook already opened under this
    /// id becomes declared when the caller names its project, and nothing here
    /// lowers it again.
    pub fn create_declared_session(
        &mut self,
        id: &str,
        project: &str,
        directory: &str,
    ) -> Result<Session, StoreError> {
        self.create_session_marked(id, project, directory, true)
    }

    /// Raises the declared flag of an existing session, provided it belongs to
    /// `project`. A session opened under another project is not what the caller
    /// named, so it is left as it was. Returns whether a row was raised.
    pub fn declare_session(&mut self, id: &str, project: &str) -> Result<bool, StoreError> {
        let project = normalize::project(project);
        let tx = self.write_transaction()?;
        let raised = tx.execute(
            "UPDATE sessions SET declared = 1 WHERE id = ?1 AND project = ?2 AND declared = 0",
            params![id, project],
        )?;
        tx.commit()?;
        Ok(raised > 0)
    }

    fn create_session_marked(
        &mut self,
        id: &str,
        project: &str,
        directory: &str,
        declared: bool,
    ) -> Result<Session, StoreError> {
        // Every local creator passes here, and the replicated one in
        // `wire.rs` refuses the same way, so an id a filesystem cannot name and
        // a project the opening block cannot hold are stopped at both doors.
        let id = id.trim();
        if let Some(refusal) = normalize::session_id_refusal(id) {
            return Err(invalid_parameter(refusal));
        }
        let project = normalize::project(project);
        if let Some(refusal) = normalize::project_refusal(&project) {
            return Err(invalid_parameter(refusal));
        }
        // Every local creator passes here, so this is where a relative
        // directory is kept out: see `recordable_directory`.
        let directory = crate::project::recordable_directory(directory);
        let tx = self.write_transaction()?;
        let created = tx.execute(
            "INSERT OR IGNORE INTO sessions (id, project, directory) VALUES (?1, ?2, ?3)",
            params![id, project, directory],
        )?;
        // After the insert and never in it, so a session that was already there
        // under another opener is raised too. Scoped to the project the caller
        // named: the row a conflicting id keeps is not the project declared.
        if declared {
            tx.execute(
                "UPDATE sessions SET declared = 1 WHERE id = ?1 AND project = ?2",
                params![id, project],
            )?;
        }
        let session = get_session_row(&tx, id)?;
        // Only when the row is new. This is an ensure, not a write: `INSERT OR
        // IGNORE` does nothing the second time and the row is returned
        // unchanged, so there is nothing for a peer to be told.
        //
        // Queueing regardless is how a journal fills up with itself. Every
        // `mem_save` that names no session lands in one stable session per
        // project and calls this first, so the store collected one identical
        // copy of that session per memory saved — 657 of them for one session
        // on a real store, in a journal larger than the memories it exists to
        // carry. A session this skips because it was already there is not lost
        // to a peer either: enrolling a project queues every session it holds.
        if created > 0 {
            enqueue_mutation(
                &tx,
                "session",
                id,
                crate::sync::OP_UPSERT,
                &session,
                &session.project,
            )?;
        }
        tx.commit()?;
        Ok(session)
    }

    /// The project's session that is still open and was last active within
    /// `within_minutes`, most recent first, skipping any id `skip` names.
    ///
    /// For the saves that name no session. The MCP server never learns the
    /// agent's own session id, so every such save used to land in the project's
    /// stable `manual-save` session: on a copy of a real store 451 of one
    /// project's 471 memories hung from it, and "Recent Sessions" showed one
    /// line for four months of work. The hooks have already opened the session
    /// the agent is in; this finds it.
    ///
    /// Activity is the latest of the session's start, its latest memory and its
    /// latest prompt, because a session opened an hour ago that was just
    /// prompted is live and one opened a minute ago that nobody touched is too.
    /// A session that has ended is a conversation that closed, and a save filed
    /// under it would be a misattribution, the same refusal `write_session`
    /// makes for a named one.
    ///
    /// `skip` is the caller's definition of a synthetic session, passed in so
    /// that the store holds no second copy of what the name looks like.
    pub fn latest_active_session(
        &self,
        project: &str,
        within_minutes: i64,
        skip: impl Fn(&str) -> bool,
    ) -> Result<Option<Session>, StoreError> {
        let project = normalize::project(project);
        let modifier = normalize::sqlite_datetime_modifier(within_minutes);
        let mut statement = self.connection.prepare(
            "SELECT id FROM (
                 SELECT s.id AS id, s.rowid AS position,
                        max(
                            ifnull(datetime(s.started_at), ''),
                            ifnull((SELECT max(datetime(o.created_at)) FROM observations o
                                    WHERE o.session_id = s.id), ''),
                            ifnull((SELECT max(datetime(p.created_at)) FROM prompts p
                                    WHERE p.session_id = s.id), '')
                        ) AS activity
                 FROM sessions s
                 WHERE s.project = ?1 AND s.ended_at IS NULL
             )
             WHERE activity >= datetime('now', ?2)
             ORDER BY activity DESC, position DESC",
        )?;
        let ids = statement
            .query_map(params![project, modifier.as_ref()], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        match ids.into_iter().find(|id| !skip(id)) {
            Some(id) => self.get_session(&id).map(Some),
            None => Ok(None),
        }
    }

    pub fn end_session(&mut self, id: &str, summary: Option<&str>) -> Result<Session, StoreError> {
        // The same rules the rest of the store's text gets: see
        // `normalize::session_summary`.
        let summary = normalize::session_summary(summary, self.config.max_observation_length);
        let tx = self.write_transaction()?;
        // `COALESCE`, not a plain assignment: the stop hook ends a session with
        // no summary, and a plain `summary = ?1` would wipe one the agent had
        // already written with `mem_session_end`. The replicated path has kept
        // the existing summary since it was written (`wire.rs`, the session
        // upsert); this one was the sibling left behind.
        let changed = tx.execute(
            "UPDATE sessions SET ended_at = datetime('now'), summary = COALESCE(?1, summary) WHERE id = ?2",
            params![summary, id],
        )?;
        if changed == 0 {
            return Err(StoreError::SessionNotFound(id.to_owned()));
        }
        let session = get_session_row(&tx, id)?;
        enqueue_session_update_tx(&tx, &session)?;
        tx.commit()?;
        Ok(session)
    }

    pub fn get_session(&self, id: &str) -> Result<Session, StoreError> {
        get_session_row(&self.connection, id)
    }

    /// The recent sessions with something to say, for a context to name.
    ///
    /// A session row is created the moment a conversation starts, because it is
    /// what anything saved later hangs off. The block prints one line per
    /// session — its summary — so a session without one has no line to print,
    /// whether it recorded nothing at all or recorded memories that are listed
    /// under their own headings.
    ///
    /// The ones with nothing to print crowd out the ones worth naming. On a
    /// real store 59 sessions of 483 were empty that way, and of the five most
    /// recent for one project, four were: the part of the opening context meant
    /// to say what has been worked on lately listed four conversations that did
    /// nothing and one that did. The filter used to run after this limit,
    /// against the five rows this returned — so those four took the places and
    /// the block printed one line where it had room for five.
    ///
    /// A summary is the row's own column or a `session_summary` memory the
    /// caller folds onto it, so both count here; a memory the graph has
    /// superseded does not, because the fold will not use it.
    ///
    /// Filtered here rather than deleted anywhere: a session without a summary
    /// is still the anchor a later save needs, and still a row the dashboard can
    /// show.
    pub fn recent_sessions(
        &self,
        project: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<SessionSummary>, StoreError> {
        let project = project.map(normalize::project);
        // No floor: this is a section of a composite answer and zero says to
        // leave it out. See `Store::timeline`, where the same `.max(1)` handed
        // back two neighbours to a caller who asked for none.
        let limit = limit.unwrap_or(5) as i64;
        let mut narrowing = Narrowing::new();
        narrowing.equals("s.project", project.as_ref());
        let limit = narrowing.bind(&limit);
        // The `EXISTS` reads the same rows `fold_session_summaries` will: an
        // unsuperseded summary of this session. Spelled with the `o` alias the
        // relation guard expects, re-scoped inside the subquery.
        let summary = crate::memory::model::SESSION_SUMMARY;
        let unsuperseded = super::relations::not_superseded();
        let mut statement = self.connection.prepare(&format!(
            "SELECT s.id, s.project, s.started_at, s.ended_at, s.summary, COUNT(o.id),
                    MAX(datetime(COALESCE(o.created_at, s.started_at)))
             FROM sessions s
             LEFT JOIN observations o ON o.session_id = s.id AND o.deleted_at IS NULL
             WHERE 1 = 1{}
             GROUP BY s.id
             HAVING trim(ifnull(s.summary, '')) <> ''
                 OR EXISTS (SELECT 1 FROM observations o
                             WHERE o.session_id = s.id
                               AND o.deleted_at IS NULL
                               AND o.type = '{summary}'
                               AND {unsuperseded})
             ORDER BY MAX(datetime(COALESCE(o.created_at, s.started_at))) DESC, s.id DESC
             LIMIT ?{limit}",
            narrowing.clauses()
        ))?;
        let rows = statement.query_map(
            rusqlite::params_from_iter(narrowing.values()),
            map_session_summary,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// One page of sessions, narrowed by project and by words.
    ///
    /// Sessions have no index of their own — only observations and prompts do —
    /// so a query here asks the question that can be answered: not "which
    /// session text contains this" but "where was this worked on", which is the
    /// question somebody scanning a store actually has. The count on each row
    /// is then of matching observations rather than of all of them, so the
    /// number says how much of the answer is in there, and the best-stocked
    /// session comes first.
    pub fn paged_sessions(
        &self,
        query: &str,
        projects: &[String],
        offset: usize,
        limit: usize,
    ) -> Result<Listing<SessionSummary>, StoreError> {
        let limit = limit.max(1) as i64;
        let offset = offset as i64;
        let fts = normalize::fts_prefix_query(query);
        let (clause, values) = Self::project_clause(projects, "s.project");
        if fts.is_empty() {
            let bound: Vec<&dyn rusqlite::ToSql> =
                values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
            let total = self.connection.query_row(
                &format!("SELECT COUNT(*) FROM sessions s WHERE 1 = 1{clause}"),
                bound.as_slice(),
                |row| row.get(0),
            )?;
            let mut statement = self.connection.prepare(&format!(
                "SELECT s.id, s.project, s.started_at, s.ended_at, s.summary, COUNT(o.id),
                        MAX(datetime(COALESCE(o.created_at, s.started_at)))
                 FROM sessions s
                 LEFT JOIN observations o ON o.session_id = s.id AND o.deleted_at IS NULL
                 WHERE 1 = 1{clause}
                 GROUP BY s.id
                 ORDER BY MAX(datetime(COALESCE(o.created_at, s.started_at))) DESC, s.id DESC
                 LIMIT ? OFFSET ?"
            ))?;
            let mut bound = bound;
            bound.push(&limit);
            bound.push(&offset);
            let rows = statement
                .query_map(bound.as_slice(), map_session_summary)?
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Listing { rows, total });
        }

        let mut bound: Vec<&dyn rusqlite::ToSql> = vec![&fts];
        bound.extend(values.iter().map(|v| v as &dyn rusqlite::ToSql));
        let matching =
            "o.id IN (SELECT rowid FROM observations_fts WHERE observations_fts MATCH ?)";
        let total = self.connection.query_row(
            &format!(
                "SELECT COUNT(DISTINCT s.id)
                 FROM sessions s
                 JOIN observations o ON o.session_id = s.id AND o.deleted_at IS NULL
                 WHERE {matching}{clause}"
            ),
            bound.as_slice(),
            |row| row.get(0),
        )?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT s.id, s.project, s.started_at, s.ended_at, s.summary, COUNT(o.id),
                    MAX(datetime(o.created_at))
             FROM sessions s
             JOIN observations o ON o.session_id = s.id AND o.deleted_at IS NULL
             WHERE {matching}{clause}
             GROUP BY s.id
             ORDER BY COUNT(o.id) DESC, MAX(datetime(o.created_at)) DESC, s.id DESC
             LIMIT ? OFFSET ?"
        ))?;
        bound.push(&limit);
        bound.push(&offset);
        let rows = statement
            .query_map(bound.as_slice(), map_session_summary)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Listing { rows, total })
    }

    /// Removes a session that holds no observations, together with its prompts.
    pub fn delete_session(&mut self, id: &str) -> Result<(), StoreError> {
        let tx = self.write_transaction()?;
        let session = get_session_row(&tx, id)?;
        let observations = tx.query_row(
            "SELECT COUNT(*) FROM observations WHERE session_id = ?1",
            [id],
            |row| row.get::<_, i64>(0),
        )?;
        if observations > 0 {
            return Err(StoreError::SessionHasObservations(
                id.to_owned(),
                observations,
            ));
        }
        let project = normalize::project(&session.project);
        for prompt in &collect_prompts_for_session_tx(&tx, id)? {
            prompts::erase_deleted_prompt_tx(
                &tx,
                &prompt.sync_id,
                &prompt.session_id,
                &prompt.project,
                None,
            )?;
            enqueue_prompt_delete_tx(&tx, prompt)?;
        }
        tx.execute("DELETE FROM prompts WHERE session_id = ?1", [id])?;
        enqueue_session_delete_tx(&tx, id, &project)?;
        tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// What deleting this session would remove, counted the way the delete
    /// walks it.
    ///
    /// For asking before deleting it. The count carried on a session row is of
    /// whatever the list was narrowed to — under a search it is the number of
    /// matches — and a confirmation that quoted that would understate what it
    /// was about to destroy.
    ///
    /// `hard` decides the second half. A soft delete leaves a tombstone and
    /// touches only what is live; a hard delete erases the tombstones too, and
    /// those are the rows somebody could still have recovered. The confirmation
    /// used to quote the live count for both, so the rows it omitted were
    /// exactly the recoverable ones.
    pub fn session_delete_counts(
        &self,
        id: &str,
        hard: bool,
    ) -> Result<super::DeleteCounts, StoreError> {
        let memories = |tombstoned: bool| -> Result<i64, StoreError> {
            Ok(self.connection.query_row(
                "SELECT COUNT(*) FROM observations
                 WHERE session_id = ?1 AND (deleted_at IS NULL) = ?2",
                params![id, !tombstoned],
                |row| row.get(0),
            )?)
        };
        let prompts = self.connection.query_row(
            "SELECT COUNT(*) FROM prompts WHERE session_id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok(super::DeleteCounts {
            live: memories(false)?,
            erased: if hard { memories(true)? } else { 0 },
            sessions: 1,
            prompts,
        })
    }

    /// Removes a session and everything it recorded.
    ///
    /// [`Self::delete_session`] refuses a session that holds observations, which
    /// is the right answer for a caller that meant to tidy up an empty one and
    /// would otherwise lose memories without asking. This is the other
    /// intention, said out loud: take the session and its contents with it.
    ///
    /// One transaction, like [`Self::delete_project`]. Deleting the observations
    /// and then the session as separate calls would leave a store half emptied
    /// if the second one failed, and the sessions list would show a session that
    /// had lost its memories.
    pub fn delete_session_and_contents(
        &mut self,
        id: &str,
        hard_delete: bool,
    ) -> Result<DeleteSessionResult, StoreError> {
        let tx = self.write_transaction()?;
        let session = get_session_row(&tx, id)?;
        let project = normalize::project(&session.project);
        let mut result = DeleteSessionResult {
            session: id.to_owned(),
            hard_delete,
            ..DeleteSessionResult::default()
        };
        let deleted_at = sqlite_now();

        let ids = query_column(&tx, "SELECT id FROM observations WHERE session_id = ?1", id)?;
        for observation_id in ids {
            let observation = get_observation_row(&tx, observation_id)?;
            // Already tombstoned and this is a soft delete: nothing to say, and
            // counting it would report work that was not done.
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
                // The memory's own project, which a replica applies the delete
                // against. The session's is not it when a memory was filed into
                // another project's session, and under that name the delete was
                // dropped whenever the session's project was not enrolled.
                observation.project.as_deref().unwrap_or_default(),
            )?;
            result.observations_deleted += 1;
        }
        if hard_delete {
            tx.execute("DELETE FROM observations WHERE session_id = ?1", [id])?;
        } else {
            tx.execute(
                "UPDATE observations SET deleted_at = ?1, updated_at = datetime('now')
                 WHERE session_id = ?2 AND deleted_at IS NULL",
                params![deleted_at, id],
            )?;
        }

        for prompt in &collect_prompts_for_session_tx(&tx, id)? {
            prompts::erase_deleted_prompt_tx(
                &tx,
                &prompt.sync_id,
                &prompt.session_id,
                &prompt.project,
                None,
            )?;
            enqueue_prompt_delete_tx(&tx, prompt)?;
        }
        result.prompts_deleted =
            tx.execute("DELETE FROM prompts WHERE session_id = ?1", [id])? as i64;

        // The session row itself only goes on a hard delete, the way a project's
        // does: soft-deleted memories still belong to the session that recorded
        // them, and a tombstone with nowhere to point is not a lighter delete
        // but a broken one.
        if hard_delete {
            enqueue_session_delete_tx(&tx, id, &project)?;
            tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        }

        tx.commit()?;
        Ok(result)
    }
}
