//! Counting, checking and moving the store as a whole.

use super::*;

impl Store {
    pub fn export(&self) -> Result<ExportData, StoreError> {
        self.export_with_project(None)
    }

    /// The whole store, or one project of it.
    ///
    /// "One project or all of them" is a single decision, and it was written
    /// out three times — here, in the `export` command, and in the sync
    /// exporter. The only difference between the branches is that naming a
    /// project validates the name, so a caller that forgot the match would
    /// silently export everything.
    pub fn export_scoped(&self, project: Option<&str>) -> Result<ExportData, StoreError> {
        match project {
            Some(project) => self.export_project(project),
            None => self.export(),
        }
    }

    pub fn export_json(&self, project: Option<&str>) -> Result<String, StoreError> {
        Ok(serde_json::to_string_pretty(&self.export_scoped(project)?)?)
    }

    pub fn import_json(&mut self, json: &str) -> Result<ImportResult, StoreError> {
        let data: ExportData = serde_json::from_str(json)?;
        self.import(&data)
    }

    pub fn import(&mut self, data: &ExportData) -> Result<ImportResult, StoreError> {
        if !data.version.is_empty() && data.version != EXPORT_FORMAT_VERSION {
            return Err(invalid_parameter(format!(
                "unsupported export format {}; this build reads {EXPORT_FORMAT_VERSION}",
                data.version
            )));
        }
        let max_length = self.config.max_observation_length;
        let tx = self.write_transaction()?;

        // The indexes are built once at the end rather than row by row on the
        // way in, which is what an import of any size spends its time on.
        //
        // Every insert below fires three triggers, and each of those tokenises
        // a title and a body through `porter unicode61` — twice for an
        // observation, which is in two indexes. Measured on a real store of
        // 4,013 memories, 486 sessions, 1,198 prompts and 326 relations:
        // 13.3 seconds with the triggers in place, 0.6 to write the rows with
        // them dropped and 0.9 to rebuild afterwards. Nine times, for the same
        // rows and the same index — 4,013 either way.
        //
        // Inside the transaction, so a failure takes the schema back with the
        // rows: SQLite rolls DDL back like anything else, and an import that
        // stopped half way must not leave a store whose indexes have no
        // triggers keeping them level.
        let dropped: Vec<&str> = crate::store::schema::FULL_TEXT_TRIGGERS
            .iter()
            .copied()
            .filter(|name| {
                tx.execute_batch(&format!("DROP TRIGGER IF EXISTS {name};"))
                    .is_ok()
            })
            .collect();

        let mut result = ImportResult::default();

        for session in &data.sessions {
            let project = normalize::project(&session.project);
            let started_at = nonempty_or_now(&session.started_at);
            result.sessions_imported += tx.execute(
                "INSERT OR IGNORE INTO sessions
                 (id, project, directory, started_at, ended_at, summary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    session.id,
                    project,
                    session.directory,
                    started_at,
                    session.ended_at,
                    session.summary
                ],
            )? as i64;
        }

        for observation in &data.observations {
            let sync_id = if observation.sync_id.trim().is_empty() {
                normalize::sync_id("obs")
            } else {
                observation.sync_id.trim().to_owned()
            };
            let title = normalize::strip_private(&observation.title);
            let content = normalize::truncate_content(
                normalize::strip_private(&observation.content),
                max_length,
            );
            let project = observation
                .project
                .as_deref()
                .map(normalize::project)
                .filter(|value| !value.is_empty());
            let scope = normalize::scope(&observation.scope);
            let topic_key = normalize::topic_key(observation.topic_key.as_deref());
            let created_at = nonempty_or_now(&observation.created_at);
            let updated_at = nonempty_or_now(&observation.updated_at);
            result.observations_imported += tx.execute(
                "INSERT INTO observations
                 (sync_id, session_id, type, title, content, tool_name, project, scope, topic_key,
                  normalized_hash, revision_count, duplicate_count, last_seen_at, review_after,
                  prompt_sync_id, pinned, created_at, updated_at, deleted_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                        ?19, ?15, ?16, ?17, ?18
                 WHERE NOT EXISTS (SELECT 1 FROM observations WHERE sync_id = ?1)",
                params![
                    sync_id,
                    observation.session_id,
                    observation.kind,
                    title,
                    content,
                    observation.tool_name,
                    project,
                    scope,
                    topic_key,
                    normalize::normalized_hash(&content),
                    observation.revision_count.max(1),
                    observation.duplicate_count.max(1),
                    observation.last_seen_at,
                    observation.review_after,
                    observation.pinned,
                    created_at,
                    updated_at,
                    observation.deleted_at,
                    observation.prompt_sync_id
                ],
            )? as i64;
        }

        for prompt in &data.prompts {
            let sync_id = if prompt.sync_id.trim().is_empty() {
                normalize::sync_id("prompt")
            } else {
                prompt.sync_id.trim().to_owned()
            };
            let content =
                normalize::truncate_content(normalize::strip_private(&prompt.content), max_length);
            let project = normalize::project(&prompt.project);
            let created_at = nonempty_or_now(&prompt.created_at);
            result.prompts_imported += tx.execute(
                "INSERT INTO prompts (sync_id, session_id, content, project, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5
                 WHERE NOT EXISTS (SELECT 1 FROM prompts WHERE sync_id = ?1)
                   AND NOT EXISTS (SELECT 1 FROM prompt_deletions WHERE sync_id = ?1)",
                params![sync_id, prompt.session_id, content, project, created_at],
            )? as i64;
        }

        // Relations last: both ends have to exist before a claim about them can
        // be stored, and the observations were only inserted a moment ago.
        //
        // One whose ends are not here is counted rather than dropped in
        // silence. An export narrowed to a project can hold a relation reaching
        // a memory that lives in another, and somebody restoring a backup
        // deserves to know the graph came back with holes.
        for relation in &data.relations {
            let sync_id = relation.sync_id.trim();
            if sync_id.is_empty()
                || relation.source_id.trim().is_empty()
                || relation.target_id.trim().is_empty()
            {
                result.relations_skipped += 1;
                continue;
            }
            let inserted = tx.execute(
                &format!(
                    "INSERT INTO memory_relations ({RELATION_INSERT_COLUMNS})
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14
                 WHERE NOT EXISTS (SELECT 1 FROM memory_relations WHERE sync_id = ?1)
                   AND EXISTS (SELECT 1 FROM observations WHERE sync_id = ?2)
                   AND EXISTS (SELECT 1 FROM observations WHERE sync_id = ?3)"
                ),
                params![
                    sync_id,
                    relation.source_id.trim(),
                    relation.target_id.trim(),
                    relation.relation,
                    relation.reason,
                    relation.evidence,
                    relation.confidence,
                    relation.judgment_status,
                    relation.marked_by_actor,
                    relation.marked_by_kind,
                    relation.marked_by_model,
                    relation.session_id,
                    nonempty_or_now(&relation.created_at),
                    nonempty_or_now(&relation.updated_at),
                ],
            )? as i64;
            if inserted == 0 && !relation_is_present(&tx, sync_id)? {
                result.relations_skipped += 1;
            } else {
                result.relations_imported += inserted;
            }
        }

        // Back, and then the indexes built from the rows that are now there.
        // In this order: a trigger restored after the rebuild would be right,
        // and one restored before it would fire on nothing, so either works —
        // what must not happen is committing without both.
        for name in dropped {
            if let Some(sql) = crate::store::schema::full_text_trigger_sql(name) {
                tx.execute_batch(sql)?;
            }
        }
        crate::store::schema::rebuild_present_indexes(&tx)?;

        tx.commit()?;
        Ok(result)
    }

    pub fn doctor(&self) -> Result<DoctorReport, StoreError> {
        self.integrity_doctor()
    }

    /// A diagnostic report, optionally narrowed to one check and one project.
    ///
    /// Both narrowings are shared by the CLI and the `mem_doctor` tool so the
    /// same request gives the same answer whichever way it is asked. An unknown
    /// code or project is refused rather than quietly matching nothing, because
    /// a diagnostic that silently reports "all clear" for a typo is worse than
    /// no diagnostic.
    pub fn doctor_scoped(
        &self,
        check: Option<&str>,
        project: Option<&str>,
    ) -> Result<(DoctorReport, Option<ProjectStats>), StoreError> {
        let mut report = self.integrity_doctor()?;
        if let Some(code) = check.map(str::trim).filter(|code| !code.is_empty()) {
            if !DoctorCheck::CODES.contains(&code) {
                return Err(invalid_parameter(format!(
                    "unknown check {code:?}; valid codes are {}",
                    DoctorCheck::CODES.join(", ")
                )));
            }
            report.checks.retain(|entry| entry.code == code);
            report.issues = report
                .checks
                .iter()
                .filter(|entry| !entry.ok)
                .filter_map(|entry| entry.detail.clone())
                .collect();
            report.healthy = report.issues.is_empty();
        }
        let stats = match project.map(normalize::project).filter(|p| !p.is_empty()) {
            Some(project) => Some(
                self.list_projects_with_stats()?
                    .into_iter()
                    .find(|stats| stats.name == project)
                    .ok_or_else(|| invalid_parameter(format!("unknown project {project:?}")))?,
            ),
            None => None,
        };
        Ok((report, stats))
    }

    pub fn integrity_doctor(&self) -> Result<DoctorReport, StoreError> {
        let mut integrity_statement = self.connection.prepare("PRAGMA integrity_check")?;
        let integrity_check = integrity_statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        let mut foreign_key_statement = self.connection.prepare("PRAGMA foreign_key_check")?;
        let foreign_key_violations = foreign_key_statement
            .query_map([], |row| {
                Ok(ForeignKeyViolation {
                    table: row.get(0)?,
                    row_id: row.get(1)?,
                    parent: row.get(2)?,
                    foreign_key_index: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut issues = Vec::new();
        // One sentence for each index, used both in the list of what is wrong
        // and as the check's own detail. Written twice, they contradicted each
        // other in the same reply: `issues` said the index could not be checked
        // while the check beside it said the index had failed.
        let check_index = |index: &str, name: &str| -> Option<String> {
            self.connection
                .execute(
                    &format!("INSERT INTO {index}({index}) VALUES ('integrity-check')"),
                    [],
                )
                .err()
                .map(|error| {
                    // The remedy only where rebuilding is the remedy: see
                    // `REBUILD_REMEDY`.
                    let verdict = why(&error);
                    match verdict {
                        "failed its integrity check" => format!(
                            "the {name} full-text index {verdict}: {error}; {REBUILD_REMEDY}"
                        ),
                        _ => format!("the {name} full-text index {verdict}: {error}"),
                    }
                })
        };
        let observation_fts = check_index("observations_fts", "observation");
        let prompt_fts = check_index("prompts_fts", "prompt");
        // The unstemmed index is searched beside the stemmed one, so it can go
        // empty or stale on its own — and a search would still answer, just
        // worse at the thing that index is for. Reported as its own checks
        // rather than as another pair of numbers, because the two counts on the
        // report have been the shape of `mem_doctor`'s output since before
        // there was a second index.
        let exact_fts = check_index("observations_exact", "unstemmed observation");
        let observations = query_count(&self.connection, "SELECT COUNT(*) FROM observations")?;
        let prompts = query_count(&self.connection, "SELECT COUNT(*) FROM prompts")?;
        // Counted from the shadow table rather than from the index itself.
        // These are external-content tables, so `SELECT COUNT(*) FROM
        // observations_fts` reads through to `observations` and agrees with it
        // even when the index holds nothing at all — which made this check
        // report a healthy store while every search came back empty.
        let observation_fts_rows = indexed_row_count(&self.connection, "observations_fts");
        let exact_fts_rows = indexed_row_count(&self.connection, "observations_exact");
        let prompt_fts_rows = indexed_row_count(&self.connection, "prompts_fts");
        // Whether every memory's hash still describes the memory.
        //
        // The hash is what dedupe compares — a save whose body matches an
        // existing one bumps that row instead of writing a second — so a hash
        // that has stopped matching its own content is a memory nothing can
        // ever be deduplicated against, silently and for good.
        //
        // It happens. A real store of 3,940 held three, all from one project on
        // one day five weeks earlier, none of them ever revised, and the text
        // their hashes were taken of is in no row of that store. Whatever wrote
        // them is gone; what was missing was anything that would notice.
        //
        // This is the same kind of check as the index ones beside it: two
        // things the store keeps that have to agree, with no error raised when
        // they stop. Reading every body costs 48 ms on that store, against a
        // command that already runs `PRAGMA integrity_check` over the whole
        // file.
        let stale_hashes = stale_hash_count(&self.connection);
        let missing_triggers = crate::store::schema::missing_full_text_triggers(&self.connection);
        let pending_mutations = query_count(
            &self.connection,
            "SELECT COUNT(*) FROM sync_mutations WHERE acked_at IS NULL",
        )?;
        let journal_mode: String = self
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        let busy_timeout_ms = self
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;

        let mut checks = Vec::new();
        let mut record = |check: DoctorCheck| {
            if let (false, Some(detail)) = (check.ok, check.detail.as_ref()) {
                issues.push(detail.clone());
            }
            checks.push(check);
        };

        record(if integrity_check.as_slice() == ["ok"] {
            DoctorCheck::passed("sqlite_integrity")
        } else {
            DoctorCheck::failed(
                "sqlite_integrity",
                format!("SQLite integrity check: {integrity_check:?}"),
            )
        });
        record(if foreign_key_violations.is_empty() {
            DoctorCheck::passed("foreign_keys")
        } else {
            DoctorCheck::failed(
                "foreign_keys",
                format!("{} foreign key violation(s)", foreign_key_violations.len()),
            )
        });
        record(match &observation_fts {
            None => DoctorCheck::passed("observation_fts_integrity"),
            Some(detail) => DoctorCheck::failed("observation_fts_integrity", detail.clone()),
        });
        record(match &prompt_fts {
            None => DoctorCheck::passed("prompt_fts_integrity"),
            Some(detail) => DoctorCheck::failed("prompt_fts_integrity", detail.clone()),
        });
        record(if observations == observation_fts_rows {
            DoctorCheck::passed("observation_fts_sync")
        } else {
            DoctorCheck::failed(
                "observation_fts_sync",
                format!(
                    "observation FTS row mismatch: table={observations}, fts={observation_fts_rows}; {REBUILD_REMEDY}"
                ),
            )
        });
        record(match &exact_fts {
            None => DoctorCheck::passed("observation_exact_fts_integrity"),
            Some(detail) => DoctorCheck::failed("observation_exact_fts_integrity", detail.clone()),
        });
        record(if observations == exact_fts_rows {
            DoctorCheck::passed("observation_exact_fts_sync")
        } else {
            DoctorCheck::failed(
                "observation_exact_fts_sync",
                format!(
                    "unstemmed observation FTS row mismatch: table={observations}, fts={exact_fts_rows}; {REBUILD_REMEDY}"
                ),
            )
        });
        record(if prompts == prompt_fts_rows {
            DoctorCheck::passed("prompt_fts_sync")
        } else {
            DoctorCheck::failed(
                "prompt_fts_sync",
                format!(
                    "prompt FTS row mismatch: table={prompts}, fts={prompt_fts_rows}; {REBUILD_REMEDY}"
                ),
            )
        });
        // Both of these are what make concurrent access safe, so a database
        // that lost them is worth reporting even though nothing is corrupt.
        record(if journal_mode.eq_ignore_ascii_case("wal") {
            DoctorCheck::passed("journal_mode")
        } else {
            DoctorCheck::failed(
                "journal_mode",
                format!("journal mode is {journal_mode}, not wal"),
            )
        });
        // Against what this store asked for, not against a number written
        // twice. The wait is a budget now — a hook sets its own, shorter than
        // the time its agent will wait before killing it — so a fixed 5000
        // here called every hook's store unhealthy. What the check is for is a
        // store that would fail instantly under a second writer, and that is
        // what zero means.
        //
        // And not against how much of that budget is left, because the budget
        // is one deadline for the whole open: the schema pass spends part of it
        // waiting out another process, and what it leaves is what the
        // connection carries. That remainder is *supposed* to be smaller.
        //
        // This allowed a flat second of it and then failed, which is not a
        // check on the store but a claim about the machine. Caught by the
        // suite: with a release build running beside it the open spent 1.5 s of
        // the five, the connection carried 3,451 ms, and `doctor` called a
        // perfectly healthy store unhealthy — twice in twenty-five runs, and it
        // would be every time on a slow disk. The same flat second that Windows
        // sleep granularity ate out of `store_wait` this morning.
        //
        // What is left is the two things that are actually wrong: a connection
        // that cannot wait at all, and one carrying more than was ever asked
        // for, which would mean something reset it behind the store's back. How
        // much of the budget survived the open is reported either way, in
        // `busy_timeout_ms`, for anyone who wants to know.
        let expected_ms = self.config.busy_timeout.as_millis() as i64;
        record(if busy_timeout_ms > 0 && busy_timeout_ms <= expected_ms {
            DoctorCheck::passed("busy_timeout")
        } else if busy_timeout_ms <= 0 {
            DoctorCheck::failed(
                "busy_timeout",
                "the store cannot wait for another writer at all".to_owned(),
            )
        } else {
            DoctorCheck::failed(
                "busy_timeout",
                format!(
                    "busy timeout is {busy_timeout_ms}ms, more than the {expected_ms}ms this store was opened with"
                ),
            )
        });

        record(if missing_triggers.is_empty() {
            DoctorCheck::passed("full_text_triggers")
        } else {
            DoctorCheck::failed(
                "full_text_triggers",
                format!(
                    "{} of the triggers that keep the full-text indexes level with the rows are missing ({}), so edits made since stopped reaching search; `leteo doctor --repair` puts them back and rebuilds",
                    missing_triggers.len(),
                    missing_triggers.join(", ")
                ),
            )
        });
        // One live memory per key, per project, per scope — and the one
        // operation that can break it says so once and then nothing does.
        //
        // `memory-model.md` §10 states the invariant and names its exception:
        // merging two projects can leave two memories under one key, because
        // each may have had its own, and the merge reports how many rather than
        // choosing which to keep. That report is a number in one reply. After
        // it, the store carries an ambiguity nothing mentions again, and the
        // cost is not theoretical: the next save under that key revises
        // whichever row the lookup reaches first and the other can never be
        // revised by its own key again. Driven on a merged store, that is
        // exactly what happens, and `doctor` called it healthy.
        //
        // No `--repair`, deliberately. Which of the two keeps the key is a
        // question about what they say, and Leteo does not read them; the
        // remedy is a person or an agent looking at both.
        // The file a person edits by hand, and the one thing here that is not
        // in the database. Serde rejects the whole document over one bad value,
        // so a `context_size` of "slimm" does not quietly fall back to the
        // default size: it discards the language, the voice and every other
        // answer in that file. `load` answers with the defaults whatever
        // happens — a hook must not fail because somebody is mid-edit — and
        // until now nothing anywhere ever said the file was being ignored.
        record(
            match self
                .config
                .database_path
                .parent()
                .map(crate::settings::ignored)
                .unwrap_or_default()
            {
                ignored if ignored.is_empty() => DoctorCheck::passed("settings_readable"),
                ignored => DoctorCheck::failed(
                    "settings_readable",
                    format!(
                        "the settings beside this database are being read past and the default used instead, on: {}; every other answer in the file still counts",
                        ignored.join("; ")
                    ),
                ),
            },
        );
        record(match shared_topic_keys(&self.connection) {
            0 => DoctorCheck::passed("topic_key_uniqueness"),
            shared => DoctorCheck::failed(
                "topic_key_uniqueness",
                format!(
                    "{shared} topic key(s) name more than one live memory in the same project and scope, so a save under one of them revises whichever comes first and leaves the rest unreachable by that key; read them with `leteo search <key>` and give one of each pair a key of its own"
                ),
            ),
        });
        // A memory filed under a word no filter can ask for.
        //
        // The category is a search filter. `mem_save` folds the close synonyms
        // and keeps anything else verbatim, which is deliberate — a word Leteo
        // does not know is still what somebody meant — and the save door says so
        // at the moment it happens. Nothing ever said it about the memories
        // already in: the hint is on the way in, and a store that collected them
        // before the hint existed had no way to find out.
        //
        // Measured on a real store of 4,121: thirty-eight, under six words —
        // `implementation`, `optimization`, `project`, `reference`, `feature`.
        // Every one of them is invisible to `mem_search` with a `type`, which is
        // how an agent narrows to decisions before proposing one.
        //
        // No `--repair`. Which of the eight a memory belongs under is a question
        // about what it says, and Leteo does not read them.
        record(match unsearchable_kinds(&self.connection).as_slice() {
            [] => DoctorCheck::passed("observation_type_searchable"),
            kinds => {
                let total: i64 = kinds.iter().map(|(_, count)| count).sum();
                let named = kinds
                    .iter()
                    .take(UNSEARCHABLE_KIND_EXAMPLES)
                    .map(|(kind, count)| format!("{kind} ({count})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let rest = kinds.len().saturating_sub(UNSEARCHABLE_KIND_EXAMPLES);
                let and_more = if rest > 0 {
                    format!(", and {rest} more word(s)")
                } else {
                    String::new()
                };
                DoctorCheck::failed(
                    "observation_type_searchable",
                    format!(
                        "{total} memories carry a type no filtered search can name: {named}{and_more}. A search narrowed by type will never return them; call mem_update with the closest of {}, or leave them where the word matters more than being found by filter",
                        crate::memory::rules::KINDS.join(", ")
                    ),
                )
            }
        });
        record(match stale_hashes {
            0 => DoctorCheck::passed("observation_hash_sync"),
            stale => DoctorCheck::failed(
                "observation_hash_sync",
                format!(
                    "{stale} memories carry a hash that no longer describes them, so nothing can be deduplicated against them; run `leteo doctor --repair` to take it again"
                ),
            ),
        });

        let healthy = issues.is_empty();
        Ok(DoctorReport {
            healthy,
            schema_version: schema_version(&self.connection)?,
            schema_supported: SCHEMA_VERSION,
            checks,
            integrity_check,
            foreign_key_violations,
            observation_fts_ok: observation_fts.is_none(),
            prompt_fts_ok: prompt_fts.is_none(),
            observations,
            observation_fts_rows,
            prompts,
            prompt_fts_rows,
            pending_mutations,
            journal_mode,
            busy_timeout_ms,
            issues,
        })
    }

    pub fn stats(&self) -> Result<Stats, StoreError> {
        let total_sessions =
            self.connection
                .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        let total_observations = self.connection.query_row(
            "SELECT COUNT(*) FROM observations WHERE deleted_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        let total_prompts =
            self.connection
                .query_row("SELECT COUNT(*) FROM prompts", [], |row| row.get(0))?;
        // The projects, newest first, without reading every memory to find out.
        //
        // Grouping the whole table by project and taking `MAX(created_at)` per
        // group is 7.7 ms of the 9.4 this call costs on a real store of 4,121
        // memories — nine tenths of it, for seventeen names. The plan says why:
        // `SEARCH observations USING INDEX idx_obs_project` walks every live row
        // through an index that holds the project and nothing else, so each row
        // costs a lookup into the table for `deleted_at` and `created_at`, and
        // then a temporary B-tree sorts the groups.
        //
        // The distinct names come out of that index without touching the table
        // at all, and each one's newest memory is one seek into
        // `idx_obs_project_order`, which is `(project, datetime(created_at)
        // DESC, id DESC)` and therefore already has that row first. Seventeen
        // seeks instead of four thousand lookups: 0.02 ms, the same seventeen
        // names in the same order.
        //
        // `MAX(datetime(...))` rather than `MAX(...)` so the ordering can use
        // that index, which is built on the expression; every other ordering in
        // this codebase already reads the column that way. The `EXISTS` says in
        // the query what the ordering would only imply — a project whose every
        // memory is deleted is not a project this lists — and the name breaks
        // ties, which the old shape left to whatever order the group scan
        // happened to produce.
        let mut statement = self.connection.prepare(PROJECTS_BY_RECENCY)?;
        let projects = statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(Stats {
            total_sessions,
            total_observations,
            total_prompts,
            projects,
        })
    }
}

/// Whether a relation with this identifier is already stored.
///
/// Distinguishes the two reasons an insert changed nothing: the relation was
/// already here, which is a re-import doing its job, and its ends are missing,
/// which is a hole in the restored graph and worth counting.
fn relation_is_present(tx: &Transaction<'_>, sync_id: &str) -> Result<bool, StoreError> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM memory_relations WHERE sync_id = ?1",
            [sync_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

/// Whether an integrity check gave a verdict or could not be made at all.
///
/// SQLite's FTS5 integrity check is run by writing a magic row into the index,
/// so anything that stops a write stops the check: a database opened read-only,
/// one another process holds, a disk with nothing left on it. Every one of
/// those used to be reported as `observations FTS integrity: <error>`, in the
/// list of what is wrong with the store — so somebody whose only problem was a
/// file permission was told their full-text index had failed, which is the kind
/// of thing people rebuild an index over.
///
/// `doctor` is the one command that has to keep failing loudly, and that only
/// works if what it says is what happened. The verdict is still unhealthy
/// either way: a store that could not be inspected is not a store that passed.
/// What changes is the sentence.
///
/// Only `SQLITE_CORRUPT` and the notice that the index is out of date with its
/// content are the index answering. Anything else is the check not running.
fn why(error: &rusqlite::Error) -> &'static str {
    let corrupt = matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if failure.code == rusqlite::ErrorCode::DatabaseCorrupt
    );
    if corrupt {
        "failed its integrity check"
    } else {
        "could not be checked"
    }
}

/// What to do about a full-text index that has gone wrong, said once.
///
/// Every check that `--repair` fixes ends with this sentence, and the two that
/// it does not fix say something else. `--repair` was built for the index
/// rebuild first — a `doctor` that could see a broken index and offer nothing
/// was the defect that motivated the flag — and then the two checks written
/// after it named the remedy while the five it exists for did not. Somebody
/// reading "observation FTS row mismatch: table=3948, fts=3940" is being told
/// what is wrong and left to guess what to do, which is the same defect wearing
/// different words.
///
/// Only where it is true. An index whose check *could not run* — a locked file,
/// a permission — is not repaired by rebuilding it, and sending somebody to
/// rebuild over a permission error is how a healthy index gets thrown away.
/// The projects a store holds, newest memory first.
///
/// A constant because the guard on it reads the plan rather than the rows: a
/// result-based test cannot tell these two shapes apart, which is exactly why
/// the old one sat here costing nine tenths of `stats` unnoticed.
pub(crate) const PROJECTS_BY_RECENCY: &str = "SELECT p.project
   FROM (SELECT DISTINCT project FROM observations WHERE project IS NOT NULL) p
  WHERE EXISTS (SELECT 1 FROM observations o
                 WHERE o.project = p.project AND o.deleted_at IS NULL)
  ORDER BY (SELECT MAX(datetime(o.created_at)) FROM observations o
             WHERE o.project = p.project AND o.deleted_at IS NULL) DESC,
           p.project";

const REBUILD_REMEDY: &str = "`leteo doctor --repair` rebuilds it from the rows it covers";

/// How many unfiled words the check names before it counts the rest.
///
/// A published limit is the limit that is applied: the words are what somebody
/// acts on, and a store that collected forty of them would otherwise put forty
/// into one line of a reply that has a size budget.
pub(crate) const UNSEARCHABLE_KIND_EXAMPLES: usize = 8;

impl Store {
    /// Rebuilds every full-text index from the rows it covers, and says what
    /// each one held before and after.
    ///
    /// `doctor` has always been able to see this break — `observation FTS row
    /// mismatch: table=3769, fts=0` — and there was nothing anybody could do
    /// about it. A store whose index has gone empty answers every search with
    /// nothing and tells the caller its words did not match, which is advice no
    /// rewording can act on, and the only path back was to write to every row
    /// until the triggers had caught up.
    ///
    /// Cheap enough to be the obvious thing to try: 483 ms for all three on a
    /// real store of 3,769 memories, after which its searches worked again.
    /// Recomputes any hash that has stopped describing its own body.
    ///
    /// Derived data, and the body is the source of truth: a hash that disagrees
    /// with the text it was taken of is wrong by definition, and taking it
    /// again is the whole repair. Returns how many were put back.
    ///
    /// Beside the index rebuild rather than in a command of its own, because
    /// they are the same act — `doctor` can see both breaks and could fix
    /// neither.
    pub fn recompute_stale_hashes(&mut self) -> Result<i64, StoreError> {
        let stale: Vec<(i64, String)> = {
            let mut statement = self.connection.prepare(
                "SELECT id, content, normalized_hash FROM observations WHERE deleted_at IS NULL",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            rows.filter_map(Result::ok)
                .filter_map(|(id, content, hash)| {
                    let taken = normalize::normalized_hash(&content);
                    (taken != hash).then_some((id, taken))
                })
                .collect()
        };
        if stale.is_empty() {
            return Ok(0);
        }
        let tx = self.write_transaction()?;
        for (id, hash) in &stale {
            tx.execute(
                "UPDATE observations SET normalized_hash = ?1 WHERE id = ?2",
                params![hash, id],
            )?;
        }
        tx.commit()?;
        Ok(stale.len() as i64)
    }

    /// Puts back any full-text trigger this database has lost.
    ///
    /// Returns their names — an empty list is the ordinary answer, and the
    /// caller rebuilds the indexes straight after either way: a trigger that
    /// was missing for a week left the index short of a week of edits, and
    /// putting the trigger back does not go and fetch them.
    pub fn restore_full_text_triggers(&self) -> Result<Vec<String>, StoreError> {
        Ok(
            crate::store::schema::restore_full_text_triggers(&self.connection)?
                .into_iter()
                .map(str::to_owned)
                .collect(),
        )
    }

    pub fn rebuild_full_text_indexes(&self) -> Result<Vec<IndexRebuild>, StoreError> {
        let before: Vec<i64> = crate::store::schema::FULL_TEXT_INDEXES
            .iter()
            .map(|index| indexed_row_count(&self.connection, index))
            .collect();
        crate::store::schema::rebuild_present_indexes(&self.connection)?;
        Ok(crate::store::schema::FULL_TEXT_INDEXES
            .iter()
            .zip(before)
            .map(|(index, rows_before)| IndexRebuild {
                index: (*index).to_owned(),
                rows_before,
                rows_after: indexed_row_count(&self.connection, index),
            })
            .collect())
    }
}

/// How many memories carry a hash that no longer describes their body.
///
/// Reads every live body, which is what makes it honest: the hash is derived
/// from the text and the only way to know they still agree is to take it again.
/// How many topic keys name more than one live memory in one project and scope.
///
/// Seven milliseconds on a store of four thousand, served by `idx_obs_topic`,
/// which is the index the revision lookup already uses — the check asks the
/// same question that lookup asks, and counts the answers it should never have
/// more than one of.
fn shared_topic_keys(connection: &Connection) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM (
                 SELECT 1 FROM observations
                  WHERE topic_key IS NOT NULL AND trim(topic_key) <> ''
                    AND deleted_at IS NULL
                  GROUP BY topic_key, project, scope
                 HAVING COUNT(*) > 1
             )",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

/// The kinds this store holds that no filtered search can name, with how many
/// memories carry each, commonest first.
///
/// Read as every distinct kind and then filtered through
/// [`crate::memory::rules::is_searchable_kind`], rather than written as a `NOT
/// IN` list in the SQL. The list of kinds is one list, and a second copy here is
/// how it would come to disagree with the door that writes them — which is the
/// mistake `REVIEW_WINDOWS` made with a third hand-written copy of three names.
fn unsearchable_kinds(connection: &Connection) -> Vec<(String, i64)> {
    let Ok(mut statement) = connection
        .prepare("SELECT type, COUNT(*) FROM observations WHERE deleted_at IS NULL GROUP BY type")
    else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    }) else {
        return Vec::new();
    };
    let mut found: Vec<(String, i64)> = rows
        .filter_map(Result::ok)
        .filter(|(kind, _)| !crate::memory::rules::is_searchable_kind(kind))
        .collect();
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    found
}

fn stale_hash_count(connection: &Connection) -> i64 {
    let Ok(mut statement) = connection
        .prepare("SELECT content, normalized_hash FROM observations WHERE deleted_at IS NULL")
    else {
        return 0;
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return 0;
    };
    rows.filter_map(Result::ok)
        .filter(|(content, hash)| &normalize::normalized_hash(content) != hash)
        .count() as i64
}
