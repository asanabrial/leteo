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
    /// several ways across machines, and `same_directory` folds separators and
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

    pub fn create_session(
        &mut self,
        id: &str,
        project: &str,
        directory: &str,
    ) -> Result<Session, StoreError> {
        let project = normalize::project(project);
        let tx = self.write_transaction()?;
        let created = tx.execute(
            "INSERT OR IGNORE INTO sessions (id, project, directory) VALUES (?1, ?2, ?3)",
            params![id, project, directory],
        )?;
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

    pub fn end_session(&mut self, id: &str, summary: Option<&str>) -> Result<Session, StoreError> {
        // The same rules the rest of the store's text gets: see
        // `normalize::session_summary`.
        let summary = normalize::session_summary(summary, self.config.max_observation_length);
        let tx = self.write_transaction()?;
        let changed = tx.execute(
            "UPDATE sessions SET ended_at = datetime('now'), summary = ?1 WHERE id = ?2",
            params![summary, id],
        )?;
        if changed == 0 {
            return Err(StoreError::SessionNotFound(id.to_owned()));
        }
        let session = get_session_row(&tx, id)?;
        enqueue_mutation(
            &tx,
            "session",
            id,
            crate::sync::OP_UPSERT,
            &session,
            &session.project,
        )?;
        tx.commit()?;
        Ok(session)
    }

    pub fn get_session(&self, id: &str) -> Result<Session, StoreError> {
        get_session_row(&self.connection, id)
    }

    /// The recent sessions that recorded something, for a context to name.
    ///
    /// A session row is created the moment a conversation starts, because it is
    /// what anything saved later hangs off. A conversation that then saves
    /// nothing and writes no summary leaves a row carrying no information at
    /// all — same project, no summary, nothing recorded — and listing it says
    /// only that somebody opened a terminal.
    ///
    /// They crowd out the ones worth naming, which is what this section is for.
    /// On a real store 59 sessions of 483 were empty that way, and of the five
    /// most recent for one project, four were: the part of the opening context
    /// meant to say what has been worked on lately listed four conversations
    /// that did nothing and one that did.
    ///
    /// Filtered here rather than deleted anywhere: an empty session is still
    /// the anchor a later save needs, and still a row the dashboard can show.
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
        let mut statement = self.connection.prepare(&format!(
            "SELECT s.id, s.project, s.started_at, s.ended_at, s.summary, COUNT(o.id),
                    MAX(datetime(COALESCE(o.created_at, s.started_at)))
             FROM sessions s
             LEFT JOIN observations o ON o.session_id = s.id AND o.deleted_at IS NULL
             WHERE 1 = 1{}
             GROUP BY s.id
             HAVING COUNT(o.id) > 0 OR trim(ifnull(s.summary, '')) <> ''
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
            enqueue_prompt_delete_tx(&tx, prompt)?;
        }
        tx.execute("DELETE FROM prompts WHERE session_id = ?1", [id])?;
        enqueue_session_delete_tx(&tx, id, &project)?;
        tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// How much a session holds: observations, then prompts.
    ///
    /// For asking before deleting it. The count carried on a session row is of
    /// whatever the list was narrowed to — under a search it is the number of
    /// matches — and a confirmation that quoted that would understate what it
    /// was about to destroy.
    pub fn session_counts(&self, id: &str) -> Result<(i64, i64), StoreError> {
        let observations = self.connection.query_row(
            "SELECT COUNT(*) FROM observations WHERE session_id = ?1 AND deleted_at IS NULL",
            params![id],
            |row| row.get(0),
        )?;
        let prompts = self.connection.query_row(
            "SELECT COUNT(*) FROM prompts WHERE session_id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        Ok((observations, prompts))
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
            tx.execute("DELETE FROM observations WHERE session_id = ?1", [id])?;
        } else {
            tx.execute(
                "UPDATE observations SET deleted_at = ?1, updated_at = datetime('now')
                 WHERE session_id = ?2 AND deleted_at IS NULL",
                params![deleted_at, id],
            )?;
        }

        for prompt in &collect_prompts_for_session_tx(&tx, id)? {
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
