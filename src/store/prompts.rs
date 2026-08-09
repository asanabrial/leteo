//! What the user actually asked, kept alongside what it produced.

use super::*;

/// How recently a question must have been asked for a memory saved outside any
/// conversation to be attributed to it.
///
/// Long enough to cover a save that comes at the end of a piece of work, and
/// short enough that a memory is never hung on a question from another sitting.
/// Beyond this the memory records no question, which is the honest answer.
///
/// Beside the rule that reads it rather than in the MCP layer, because both
/// doors into the table now attribute and a window that lived on one of them
/// would be a second copy for the other to get wrong.
pub const PROMPT_ATTRIBUTION_MINUTES: i64 = 30;

impl Store {
    pub fn get_prompt(&self, id: i64) -> Result<Prompt, StoreError> {
        self.connection
            .query_row(
                &format!("SELECT {PROMPT_COLUMNS} FROM prompts WHERE id = ?1"),
                params![id],
                map_prompt,
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => StoreError::PromptNotFound(id),
                other => StoreError::from(other),
            })
    }

    /// One page of prompts, narrowed by project and by words.
    pub fn paged_prompts(
        &self,
        query: &str,
        projects: &[String],
        offset: usize,
        limit: usize,
    ) -> Result<Listing<Prompt>, StoreError> {
        let limit = limit.max(1) as i64;
        let offset = offset as i64;
        let fts = normalize::fts_prefix_query(query);
        if fts.is_empty() {
            let (clause, values) = Self::project_clause(projects, "project");
            let bound: Vec<&dyn rusqlite::ToSql> =
                values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
            let total = self.connection.query_row(
                &format!("SELECT COUNT(*) FROM prompts WHERE 1 = 1{clause}"),
                bound.as_slice(),
                |row| row.get(0),
            )?;
            let mut statement = self.connection.prepare(&format!(
                "SELECT {PROMPT_COLUMNS} FROM prompts
                 WHERE 1 = 1{clause}
                 ORDER BY datetime(created_at) DESC, id DESC LIMIT ? OFFSET ?"
            ))?;
            let mut bound = bound;
            bound.push(&limit);
            bound.push(&offset);
            let rows = statement
                .query_map(bound.as_slice(), map_prompt)?
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Listing { rows, total });
        }

        let (clause, values) = Self::project_clause(projects, "p.project");
        let mut bound: Vec<&dyn rusqlite::ToSql> = vec![&fts];
        bound.extend(values.iter().map(|v| v as &dyn rusqlite::ToSql));
        let total = self.connection.query_row(
            &format!(
                "SELECT COUNT(*) FROM prompts_fts fts CROSS JOIN prompts p ON p.id = fts.rowid
                 WHERE prompts_fts MATCH ?{clause}"
            ),
            bound.as_slice(),
            |row| row.get(0),
        )?;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {PROMPT_COLUMNS_JOINED}
             FROM prompts_fts fts CROSS JOIN prompts p ON p.id = fts.rowid
             WHERE prompts_fts MATCH ?{clause}
             ORDER BY rank LIMIT ? OFFSET ?"
        ))?;
        bound.push(&limit);
        bound.push(&offset);
        let rows = statement
            .query_map(bound.as_slice(), map_prompt)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Listing { rows, total })
    }

    pub fn add_prompt(&mut self, input: AddPrompt) -> Result<Prompt, StoreError> {
        let (content, project) = normalize::prompt_fields(
            &input.content,
            input.project.as_deref(),
            self.config.max_observation_length,
        );
        // The same door a memory goes through, one field wide. Checked after
        // the redaction rather than before it, so a prompt that was *all*
        // private is still a prompt: it says a question was asked and that its
        // words are not for keeping.
        if let Some(refusal) = crate::memory::rules::refuse_prompt(&content) {
            return Err(invalid_parameter(refusal.message()));
        }
        let sync_id = normalize::sync_id("prompt");
        let tx = self.write_transaction()?;
        ensure_session_tx(&tx, &input.session_id)?;
        // The same words, in the same conversation, in the same second or two:
        // that is one prompt recorded twice, not somebody typing fast.
        //
        // A memory is a fact and two identical ones are one; a prompt is an
        // event and two identical events are usually two — somebody does say
        // "continue" again. So the window here is seconds rather than the
        // fifteen minutes observations use, and it only catches an echo.
        //
        // It exists because an echo is not hypothetical. Two lifecycle hooks
        // were registered for one event and every prompt was stored twice for
        // as long as that lasted. The registration is guarded now, but the
        // store is the one thing both callers share, so this is where a repeat
        // can be refused whatever causes it.
        if let Some(echo) = echoed_prompt(&tx, &input.session_id, &content)? {
            return Ok(echo);
        }
        tx.execute(
            "INSERT INTO prompts (sync_id, session_id, content, project) VALUES (?1, ?2, ?3, ?4)",
            params![sync_id, input.session_id, content, project],
        )?;
        let id = tx.last_insert_rowid();
        let prompt = tx.query_row(
            &format!("SELECT {PROMPT_COLUMNS} FROM prompts WHERE id = ?1"),
            [id],
            |row| {
                Ok(Prompt {
                    id: row.get(0)?,
                    sync_id: row.get(1)?,
                    session_id: row.get(2)?,
                    content: row.get(3)?,
                    project: row.get(4)?,
                    created_at: row.get(5)?,
                })
            },
        )?;
        enqueue_mutation(
            &tx,
            "prompt",
            &prompt.sync_id,
            crate::sync::OP_UPSERT,
            &prompt,
            &prompt.project,
        )?;
        tx.commit()?;
        Ok(prompt)
    }

    /// The lookup behind [`Store::latest_session_prompt_sync_id`], named so
    /// that something can hold it to the one column it needs.
    ///
    /// Spelled out here rather than inline because of what cannot be tested any
    /// other way. Widening this `SELECT` returns exactly the same answer, so
    /// every behavioural test passes either way — the only difference is that
    /// SQLite reads `content` off disk, which is the largest column in the
    /// table and holds a whole prompt. A mutation that did precisely that
    /// survived a sweep, and it was right to: there was nowhere for a test to
    /// look. The test beside it reads the query text, which is the only place
    /// this invariant exists at all.
    pub(super) const LATEST_PROMPT_SYNC_ID: &'static str = "SELECT ifnull(sync_id, '') FROM prompts
                 WHERE session_id = ?1 AND trim(ifnull(sync_id, '')) <> ''
                 ORDER BY id DESC LIMIT 1";

    /// The last thing somebody typed in this session, if anything.
    ///
    /// For linking a memory to the question that produced it. The MCP server
    /// keeps that link in memory, set by `mem_save_prompt` — but prompts are
    /// captured by the `user-prompt-submit` hook, which is a **separate
    /// process**, so the server never learns about them and the in-memory
    /// context stays `None`. A real store of 3,550 memories carried
    /// `prompt_sync_id` on exactly none of them.
    ///
    /// Scoped to the session rather than the project, because a session is one
    /// conversation and that is the only scope where "the last question" means
    /// anything. Across a project it would attribute a memory to whatever
    /// somebody asked in an unrelated window.
    ///
    /// One column, not the whole row. The caller links a memory to a prompt and
    /// needs the identifier; the body is the largest column in the table and is
    /// read by nobody here. The same mistake cost the prompt hook 58 kB of
    /// content to print three lines, which is recorded on [`MemoryRef`] — this
    /// runs on every save, so it is the hotter of the two.
    ///
    /// `id DESC` alone orders it. `created_at` is a text timestamp in one
    /// format, written by one writer, so the row with the highest id is the
    /// last one written — and `datetime(created_at)` is a function call that
    /// no index can answer, which turns a lookup into a sort.
    ///
    /// [`MemoryRef`]: crate::memory::model::MemoryRef
    pub fn latest_session_prompt_sync_id(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, StoreError> {
        self.connection
            .query_row(Self::LATEST_PROMPT_SYNC_ID, params![session_id], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(StoreError::from)
    }

    /// The last thing typed in this project, if it was typed just now.
    ///
    /// For the saves that land in no conversation at all. A memory saved
    /// without a `session_id` goes to a stable per-project bucket, and the hook
    /// writes prompts under the agent's own session — so asking that bucket
    /// what was last asked in it returns nothing, for ever. On a real store
    /// that is 1,081 memories of 3,682 against 4 prompts of 817.
    ///
    /// The window is what keeps this honest. Scoped to the project alone it
    /// would attribute today's memory to a question from last week, which is
    /// worse than admitting there is no link: a wrong answer to "what was I
    /// asked when I learned this" cannot be told from a right one.
    ///
    /// `datetime(created_at)` rather than a string comparison, because the
    /// modifier arrives as one — the column is written in a single format by a
    /// single writer, so this is a scan of the tail of one index rather than of
    /// the table.
    pub fn latest_project_prompt_sync_id(
        &self,
        project: &str,
        within_minutes: i64,
    ) -> Result<Option<String>, StoreError> {
        let project = normalize::project(project);
        let modifier = normalize::sqlite_datetime_modifier(within_minutes);
        self.connection
            .query_row(
                "SELECT ifnull(sync_id, '') FROM prompts
                 WHERE project = ?1 AND trim(ifnull(sync_id, '')) <> ''
                   AND datetime(created_at) >= datetime('now', ?2)
                 ORDER BY id DESC LIMIT 1",
                params![project, modifier.as_ref()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// The question a save should be attributed to, by the two rules above.
    ///
    /// Both doors write to one table and only one of them ever asked this.
    /// `mem_save` reached for the session's last prompt and then, for a save
    /// that named no session, the project's — sixty lines of reasoning about
    /// which attribution is safe — while `leteo save` wrote `None` with nothing
    /// said about why. A memory saved from a terminal is in the same
    /// conversation as one saved through the tool when it names the same
    /// session, and it answers the same question.
    ///
    /// `named` is whether the caller chose the session. A caller who did has
    /// named one conversation, so a question from a different one is not what
    /// their memory answers, and the project fallback is not offered to them.
    ///
    /// Returns `None` rather than failing: a memory is worth saving when the
    /// question behind it is unknown, which is why every step here is
    /// best-effort.
    pub fn prompt_behind_a_save(
        &self,
        session_id: &str,
        project: &str,
        named: bool,
    ) -> Option<String> {
        self.latest_session_prompt_sync_id(session_id)
            .ok()
            .flatten()
            .or_else(|| {
                (!named)
                    .then(|| {
                        self.latest_project_prompt_sync_id(project, PROMPT_ATTRIBUTION_MINUTES)
                            .ok()
                            .flatten()
                    })
                    .flatten()
            })
    }

    pub fn recent_prompts(
        &self,
        project: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Prompt>, StoreError> {
        let project = project.map(normalize::project);
        let limit = limit.unwrap_or(10).max(1) as i64;
        let mut narrowing = Narrowing::new();
        narrowing.equals("project", project.as_ref());
        let limit = narrowing.bind(&limit);
        // `WHERE 1 = 1` so the first narrowing's ` AND ` has something to
        // attach to, and so the clause-free form is still a legal statement.
        let mut statement = self.connection.prepare(&format!(
            "SELECT {PROMPT_COLUMNS}
             FROM prompts WHERE 1 = 1{}
             ORDER BY datetime(created_at) DESC, id DESC LIMIT ?{limit}",
            narrowing.clauses()
        ))?;
        let rows =
            statement.query_map(rusqlite::params_from_iter(narrowing.values()), map_prompt)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The recent prompts, most recent copy of each thing asked.
    ///
    /// What a context is built from, where [`Store::recent_prompts`] is the
    /// listing. The difference is worth the second query: these are the ten
    /// that go into what a session opens with, and taken by time alone anything
    /// asked twice spends two of the ten places. The shapes that repeat are the
    /// ordinary ones — a slash command, "carry on", a question retyped after a
    /// failure, something running on a timer — so it is not a corner case: on a
    /// real store five projects of twelve had three or four distinct prompts
    /// among their last ten, and the section meant to say what somebody has
    /// been working on said one sentence several times over.
    ///
    /// Twice over, because one comparison cannot see both shapes. The SQL
    /// groups by the text exactly, which is what catches a question genuinely
    /// retyped; the pass in Rust groups by `normalize::prompt_core`, which is
    /// what catches the same question with a slash command in front of it. The
    /// note above named a slash command as the first shape that repeats and
    /// then compared exactly, so `/loop find bugs` and `find bugs` — one
    /// request, typed once to start a loop and again by the loop — spent two
    /// places and, being the long ones, 444 bytes of that section's 1,278.
    ///
    /// The echo guard on the way in does not cover this and should not. That
    /// one refuses the same words in the same conversation within seconds,
    /// because two hooks firing on one event is not two questions. This is the
    /// same question genuinely asked again an hour later: a real event, worth
    /// storing, and not worth *listing* twice.
    pub fn recent_distinct_prompts(
        &self,
        project: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<Prompt>, StoreError> {
        let project = project.map(normalize::project);
        // No floor, for the reason `recent_sessions` has none: a section of a
        // composite answer, and zero asks for none of it.
        let wanted = limit.unwrap_or(10);
        // Read more than are wanted, because the second pass throws some away.
        //
        // The SQL groups by the text exactly; `normalize::prompt_core` catches
        // what that cannot — the same question with a slash command in front of
        // it, or with different spacing or case. Three times the budget covers
        // a stretch where every prompt is a variant of one question, costs
        // twenty more rows of a table read by an index, and where it does not
        // cover it the answer is short rather than wrong.
        let over_fetch = (wanted.saturating_mul(3)).min(120) as i64;
        let mut narrowing = Narrowing::new();
        narrowing.equals("project", project.as_ref());
        let limit = narrowing.bind(&over_fetch);
        // And nothing blank. The door refuses those now, but a store that has
        // been running a while already holds them — eleven on a real one — and
        // a line reading "somebody asked:" with nothing after it spends one of
        // the ten places on saying that a question happened.
        let mut statement = self.connection.prepare(&format!(
            "SELECT {PROMPT_COLUMNS}
             FROM prompts WHERE trim(ifnull(content, '')) <> '' AND id IN (
                 SELECT MAX(id) FROM prompts WHERE 1 = 1{}
                  GROUP BY content
             )
             ORDER BY datetime(created_at) DESC, id DESC LIMIT ?{limit}",
            narrowing.clauses()
        ))?;
        let rows =
            statement.query_map(rusqlite::params_from_iter(narrowing.values()), map_prompt)?;
        let mut seen = std::collections::HashSet::new();
        let mut kept = Vec::with_capacity(wanted);
        for prompt in rows {
            let prompt = prompt?;
            if seen.insert(normalize::prompt_core(&prompt.content)) {
                kept.push(prompt);
                if kept.len() == wanted {
                    break;
                }
            }
        }
        Ok(kept)
    }

    pub fn delete_prompt(&mut self, id: i64) -> Result<(), StoreError> {
        let tx = self.write_transaction()?;
        let prompt = get_prompt_tx(&tx, id)?;
        let deleted_at = sqlite_now();
        let changed = tx.execute("DELETE FROM prompts WHERE id = ?1", [id])?;
        if changed == 0 {
            return Err(StoreError::PromptNotFound(id));
        }
        tx.execute(
            "INSERT INTO prompt_deletions (sync_id, session_id, project, deleted_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(sync_id) DO UPDATE SET session_id = excluded.session_id,
                 project = excluded.project, deleted_at = excluded.deleted_at",
            params![
                prompt.sync_id,
                prompt.session_id,
                prompt.project,
                deleted_at
            ],
        )?;
        let payload = serde_json::json!({
            "sync_id": prompt.sync_id,
            "session_id": prompt.session_id,
            "project": prompt.project,
            "deleted": true,
            "deleted_at": deleted_at,
            "hard_delete": true,
        });
        enqueue_mutation(
            &tx,
            "prompt",
            &prompt.sync_id,
            crate::sync::OP_DELETE,
            &payload,
            &prompt.project,
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// Seconds within which the same words in the same session are one prompt.
///
/// SQLite stamps `created_at` to the second, so this is as fine as the clock
/// gets. Two is enough for two hooks racing on one event and far too short for
/// anybody to have typed the same thing again.
const PROMPT_ECHO_SECONDS: i64 = 2;

/// The prompt this one would be a second copy of, if there is one.
fn echoed_prompt(
    tx: &Transaction<'_>,
    session_id: &str,
    content: &str,
) -> Result<Option<Prompt>, StoreError> {
    tx.query_row(
        &format!(
            "SELECT {PROMPT_COLUMNS} FROM prompts
             WHERE session_id = ?1 AND content = ?2
               AND datetime(created_at) >= datetime('now', ?3)
             ORDER BY id DESC LIMIT 1"
        ),
        params![
            session_id,
            content,
            format!("-{PROMPT_ECHO_SECONDS} seconds")
        ],
        map_prompt,
    )
    .optional()
    .map_err(StoreError::from)
}
