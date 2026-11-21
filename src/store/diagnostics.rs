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
        if !data.version.is_empty() && !READABLE_EXPORT_VERSIONS.contains(&data.version.as_str()) {
            return Err(invalid_parameter(format!(
                "unsupported export format {}; this build reads {}",
                data.version,
                READABLE_EXPORT_VERSIONS.join(" or ")
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
            // The type is folded the same way the local and replicated doors
            // fold it. An export written by a build that stored the caller's
            // word verbatim — `manual`, `bug` — restores a memory filed where
            // no filtered search asks for it, which `doctor` then reports as an
            // unhealthy store. Reading the format back has to apply the rules
            // writing it applies.
            let kind = normalize::kind(&observation.kind);
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
                    kind,
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

        // The version history of the memories just restored. Applied through the
        // same helper the live and replicated paths use, so the retention bound
        // and the idempotent identity stay one implementation rather than a
        // third copy that can drift.
        for version in &data.observation_versions {
            if version.observation_sync_id.trim().is_empty() || version.revision <= 0 {
                continue;
            }
            let title = normalize::title(&version.title, max_length);
            let content =
                normalize::truncate_content(normalize::strip_private(&version.content), max_length);
            let replaced_at = nonempty_or_now(&version.replaced_at);
            result.observation_versions_imported +=
                crate::store::observations::snapshot_observation_version_tx(
                    &tx,
                    version.observation_sync_id.trim(),
                    version.revision,
                    &title,
                    &content,
                    &replaced_at,
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

        // Engram 0.2.0's prompt tombstones, which Leteo has a home for:
        // `prompt_deletions` is the table `mem_delete` writes and the prompt
        // insert above already consults, so a prompt the source had deleted
        // comes back deleted rather than resurrected. `source_inbox_id`, which
        // 0.2.0 also carries here, has no column in this model and is dropped
        // with the rest of the fields Leteo does not hold.
        for tombstone in &data.prompt_tombstones {
            let sync_id = tombstone.sync_id.trim();
            if sync_id.is_empty() {
                continue;
            }
            let project = tombstone
                .project
                .as_deref()
                .map(normalize::project)
                .unwrap_or_default();
            result.prompt_tombstones_imported += tx.execute(
                "INSERT OR IGNORE INTO prompt_deletions (sync_id, session_id, project, deleted_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    sync_id,
                    tombstone.session_id,
                    project,
                    nonempty_or_now(&tombstone.deleted_at)
                ],
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

        // The indexes built from the rows that are now there, and then the
        // triggers back.
        //
        // In this order and not the other: the rebuild begins by writing the
        // stems of every memory that has none, and with the triggers already
        // restored each of those writes would also be indexed one row at a
        // time, which is the cost dropping them was for, ahead of a rebuild
        // that replaces it. What must not happen is committing without both.
        crate::store::schema::rebuild_present_indexes(&tx)?;
        for name in dropped {
            if let Some(sql) = crate::store::schema::full_text_trigger_sql(name) {
                tx.execute_batch(sql)?;
            }
        }

        tx.commit()?;
        // A memory is given a vector where it is written, and an import is a
        // write like any other: the rows it restored get vectors now, up to the
        // bound, and the rest are left for the background backfill. Bounded,
        // because an import can be the whole store and its reply must not wait
        // on making vectors for all of it.
        if let Err(error) = self.backfill_vectors(super::semantic_stage::BACKFILL_BUDGET) {
            tracing::warn!(%error, "the semantic stage could not keep vectors for what was imported");
        }
        Ok(result)
    }

    pub fn doctor(&self) -> Result<DoctorReport, StoreError> {
        self.doctor_run(None, None)
    }

    /// A diagnostic report, optionally narrowed to one check and one project.
    ///
    /// Both narrowings are shared by the CLI and the `mem_doctor` tool so the
    /// same request gives the same answer whichever way it is asked. An unknown
    /// code or project is refused rather than quietly matching nothing, because
    /// a diagnostic that silently reports "all clear" for a typo is worse than
    /// no diagnostic.
    ///
    /// Naming a check runs **only** that check. That is the whole of `--check`:
    /// asking about `busy_timeout` should not pay for `PRAGMA integrity_check`,
    /// which on a 96 MB store is several seconds the one answer does not need.
    /// What a check that did not run would have measured is left absent rather
    /// than zeroed, so no report reads as "the index holds nothing" when the
    /// index was never looked at.
    pub fn doctor_scoped(
        &self,
        check: Option<&str>,
        project: Option<&str>,
    ) -> Result<(DoctorReport, Option<ProjectStats>), StoreError> {
        self.doctor_scoped_citing(check, project, None)
    }

    /// [`Store::doctor_scoped`], with the means to run the opt-in checks.
    ///
    /// `citations` is the directory and project the check that reads the
    /// repository is to look at. With it, an unfiltered report carries that check
    /// as well; without it the check is not part of the report, and naming it
    /// with `check` is an error — it has nowhere to look, and an empty report
    /// would read as every citation resolving.
    pub fn doctor_scoped_citing(
        &self,
        check: Option<&str>,
        project: Option<&str>,
        citations: Option<&crate::citations::Scope>,
    ) -> Result<(DoctorReport, Option<ProjectStats>), StoreError> {
        let report = match check.map(str::trim).filter(|code| !code.is_empty()) {
            Some(code) => {
                if !DoctorCheck::CODES.contains(&code) {
                    return Err(invalid_parameter(format!(
                        "unknown check {code:?}; valid codes are {}",
                        DoctorCheck::CODES.join(", ")
                    )));
                }
                if DoctorCheck::OPT_IN.contains(&code) && citations.is_none() {
                    return Err(invalid_parameter(format!(
                        "check {code:?} reads the repository of the directory it is run from and runs git, so only `leteo doctor --check {code}` can ask for it; it is not part of an unfiltered report"
                    )));
                }
                self.doctor_run(Some(code), citations)?
            }
            None => self.doctor_run(None, citations)?,
        };
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

    /// One diagnostic pass, over every check or over a single one.
    ///
    /// `only` is the requested code and narrows the run to it. Every check is
    /// one `if want(..)` block over the same report, so an added check runs
    /// under `doctor` and is selectable under `--check` with no second list to
    /// keep in step.
    ///
    /// The counts and pragmas below are gathered first and unconditionally. On
    /// a 96 MB file they cost a few milliseconds against the seconds `PRAGMA
    /// integrity_check` costs, and carrying them only when their own check ran
    /// would make `--check` answer a different question than `doctor` for no
    /// time saved.
    fn doctor_run(
        &self,
        only: Option<&str>,
        citations: Option<&crate::citations::Scope>,
    ) -> Result<DoctorReport, StoreError> {
        let want = |code: &str| only.is_none_or(|only| only == code);

        let observations = query_count(&self.connection, "SELECT COUNT(*) FROM observations")?;
        let prompts = query_count(&self.connection, "SELECT COUNT(*) FROM prompts")?;
        // Counted from the shadow table rather than from the index itself.
        // These are external-content tables, so `SELECT COUNT(*) FROM
        // observations_fts` reads through to `observations` and agrees with it
        // even when the index holds nothing at all — which made the check
        // report a healthy store while every search came back empty.
        let observation_fts_rows = indexed_row_count(&self.connection, "observations_fts");
        let prompt_fts_rows = indexed_row_count(&self.connection, "prompts_fts");
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
        let mut issues = Vec::new();
        let mut record = |check: DoctorCheck| {
            // Only errors are issues. A warning is preserved in `checks` and
            // leaves `healthy` alone, which is the whole difference between the
            // two severities.
            if !check.ok && check.severity == DoctorSeverity::Error {
                issues.extend(check.detail.iter().cloned());
            }
            checks.push(check);
        };

        // Whether an index check gave a verdict or could not be made at all.
        //
        // SQLite's FTS5 integrity check is run by writing a magic row into the
        // index, so anything that stops a write stops the check: a database
        // opened read-only, one another process holds, a disk with nothing left
        // on it. Every one of those used to be reported as `observations FTS
        // integrity: <error>`, in the list of what is wrong with the store — so
        // somebody whose only problem was a file permission was told their
        // full-text index had failed, which is the kind of thing people rebuild
        // an index over.
        //
        // `doctor` is the one command that has to keep failing loudly, and that
        // only works if what it says is what happened. The verdict is still
        // unhealthy either way: a store that could not be inspected is not a
        // store that passed. What changes is the sentence.
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

        // The aggregates that only their own check can answer, present exactly
        // when that check ran.
        let mut integrity_check: Option<Vec<String>> = None;
        let mut foreign_key_violations: Option<Vec<ForeignKeyViolation>> = None;
        let mut observation_fts_ok: Option<bool> = None;
        let mut prompt_fts_ok: Option<bool> = None;

        if want("sqlite_integrity") {
            let verdict = {
                let mut statement = self.connection.prepare("PRAGMA integrity_check")?;
                statement
                    .query_map([], |row| row.get(0))?
                    .collect::<Result<Vec<String>, _>>()?
            };
            record(if verdict.as_slice() == ["ok"] {
                DoctorCheck::passed("sqlite_integrity")
            } else {
                DoctorCheck::failed(
                    "sqlite_integrity",
                    format!("SQLite integrity check: {verdict:?}"),
                )
            });
            integrity_check = Some(verdict);
        }
        if want("foreign_keys") {
            let violations = {
                let mut statement = self.connection.prepare("PRAGMA foreign_key_check")?;
                statement
                    .query_map([], |row| {
                        Ok(ForeignKeyViolation {
                            table: row.get(0)?,
                            row_id: row.get(1)?,
                            parent: row.get(2)?,
                            foreign_key_index: row.get(3)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?
            };
            record(if violations.is_empty() {
                DoctorCheck::passed("foreign_keys")
            } else {
                DoctorCheck::failed(
                    "foreign_keys",
                    format!("{} foreign key violation(s)", violations.len()),
                )
            });
            foreign_key_violations = Some(violations);
        }
        if want("observation_fts_integrity") {
            let detail = check_index("observations_fts", "observation");
            record(match &detail {
                None => DoctorCheck::passed("observation_fts_integrity"),
                Some(detail) => DoctorCheck::failed("observation_fts_integrity", detail.clone()),
            });
            observation_fts_ok = Some(detail.is_none());
        }
        if want("prompt_fts_integrity") {
            let detail = check_index("prompts_fts", "prompt");
            record(match &detail {
                None => DoctorCheck::passed("prompt_fts_integrity"),
                Some(detail) => DoctorCheck::failed("prompt_fts_integrity", detail.clone()),
            });
            prompt_fts_ok = Some(detail.is_none());
        }
        if want("observation_fts_sync") {
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
        }
        if want("observation_exact_fts_integrity") {
            let detail = check_index("observations_exact", "unstemmed observation");
            record(match &detail {
                None => DoctorCheck::passed("observation_exact_fts_integrity"),
                Some(detail) => {
                    DoctorCheck::failed("observation_exact_fts_integrity", detail.clone())
                }
            });
        }
        if want("observation_exact_fts_sync") {
            let exact_fts_rows = indexed_row_count(&self.connection, "observations_exact");
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
        }
        if want("observation_stems_fts_integrity") {
            let detail = check_index("observations_stemmed", "Snowball observation");
            record(match &detail {
                None => DoctorCheck::passed("observation_stems_fts_integrity"),
                Some(detail) => {
                    DoctorCheck::failed("observation_stems_fts_integrity", detail.clone())
                }
            });
        }
        if want("observation_stems_sync") {
            // Two counts for this one, because it is two things that can drift:
            // the stems table against the memories, and the index against the
            // stems.
            //
            // A count that failed is not a count of -1. Both sides used to fall
            // back to that sentinel, so two failures were equal to each other and
            // to the index's own sentinel, and the check passed.
            let stems_rows =
                query_count(&self.connection, "SELECT COUNT(*) FROM observation_stems");
            // Only a memory with an integer id can have stems, so only those
            // are owed any. A database that predates the baseline can carry a
            // TEXT id, which no repair can give a row here, and counting it
            // would report a mismatch that nothing could clear.
            let stemmable = query_count(
                &self.connection,
                "SELECT COUNT(*) FROM observations WHERE typeof(id) = 'integer'",
            );
            let stems_fts_rows = indexed_row_count(&self.connection, "observations_stemmed");
            record(match (stemmable, stems_rows) {
                (Err(error), _) | (_, Err(error)) => could_not_run(
                    "observation_stems_sync",
                    "the Snowball observation index row counts",
                    &error,
                ),
                (Ok(stemmable), Ok(stems_rows)) => {
                    if stemmable == stems_rows && stems_rows == stems_fts_rows {
                        DoctorCheck::passed("observation_stems_sync")
                    } else {
                        DoctorCheck::failed(
                            "observation_stems_sync",
                            format!(
                                "Snowball observation index row mismatch: memories that can have stems={stemmable}, stems={stems_rows}, fts={stems_fts_rows}; {REBUILD_REMEDY}"
                            ),
                        )
                    }
                }
            });
        }
        if want("prompt_fts_sync") {
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
        }
        if want("journal_mode") {
            // What makes concurrent access safe, so a database that lost it is
            // worth reporting even though nothing is corrupt — and worth a
            // warning rather than an error, because a store used by one
            // process at a time still works.
            record(if journal_mode.eq_ignore_ascii_case("wal") {
                DoctorCheck::passed("journal_mode")
            } else {
                DoctorCheck::warned(
                    "journal_mode",
                    format!("journal mode is {journal_mode}, not wal"),
                )
            });
        }
        if want("busy_timeout") {
            // Against what this store asked for, not against a number written
            // twice. The wait is a budget now — a hook sets its own, shorter
            // than the time its agent will wait before killing it — so a fixed
            // 5000 here called every hook's store unhealthy. What the check is
            // for is a store that would fail instantly under a second writer,
            // and that is what zero means.
            //
            // And not against how much of that budget is left, because the
            // budget is one deadline for the whole open: the schema pass spends
            // part of it waiting out another process, and what it leaves is what
            // the connection carries. That remainder is *supposed* to be
            // smaller.
            //
            // This allowed a flat second of it and then failed, which is not a
            // check on the store but a claim about the machine. Caught by the
            // suite: with a release build running beside it the open spent 1.5 s
            // of the five, the connection carried 3,451 ms, and `doctor` called
            // a perfectly healthy store unhealthy — twice in twenty-five runs,
            // and it would be every time on a slow disk. The same flat second
            // that Windows sleep granularity ate out of `store_wait` this
            // morning.
            //
            // What is left is the two things that are actually wrong: a
            // connection that cannot wait at all, and one carrying more than was
            // ever asked for, which would mean something reset it behind the
            // store's back. How much of the budget survived the open is reported
            // either way, in `busy_timeout_ms`, for anyone who wants to know.
            let expected_ms = self.config.busy_timeout.as_millis() as i64;
            record(if busy_timeout_ms > 0 && busy_timeout_ms <= expected_ms {
                DoctorCheck::passed("busy_timeout")
            } else if busy_timeout_ms <= 0 {
                DoctorCheck::warned(
                    "busy_timeout",
                    "the store cannot wait for another writer at all".to_owned(),
                )
            } else {
                DoctorCheck::warned(
                    "busy_timeout",
                    format!(
                        "busy timeout is {busy_timeout_ms}ms, more than the {expected_ms}ms this store was opened with"
                    ),
                )
            });
        }
        if want("full_text_triggers") {
            record(
                match crate::store::schema::missing_full_text_triggers(&self.connection) {
                    Err(error) => {
                        could_not_run("full_text_triggers", "the full-text triggers", &error)
                    }
                    Ok(missing) if missing.is_empty() => DoctorCheck::passed("full_text_triggers"),
                    Ok(missing) => DoctorCheck::failed(
                        "full_text_triggers",
                        format!(
                            "{} of the triggers that keep the full-text indexes level with the rows are missing ({}), so edits made since stopped reaching search; `leteo doctor --repair` puts them back and rebuilds",
                            missing.len(),
                            missing.join(", ")
                        ),
                    ),
                },
            );
        }
        if want("settings_readable") {
            // The file a person edits by hand, and the one thing here that is
            // not in the database. Serde rejects the whole document over one bad
            // value, so a `context_size` of "slimm" does not quietly fall back
            // to the default size: it discards the language, the voice and every
            // other answer in that file. `load` answers with the defaults
            // whatever happens — a hook must not fail because somebody is
            // mid-edit — and until now nothing anywhere ever said the file was
            // being ignored.
            let ignored = self
                .config
                .database_path
                .parent()
                .map(crate::settings::ignored)
                .unwrap_or_default();
            record(if ignored.is_empty() {
                DoctorCheck::passed("settings_readable")
            } else {
                DoctorCheck::warned(
                    "settings_readable",
                    format!(
                        "the settings beside this database are being read past and the default used instead, on: {}; every other answer in the file still counts",
                        ignored.join("; ")
                    ),
                )
            });
        }
        if want("memory_language") {
            // The rows already in the store keep the language they were written
            // under, and which language that is is a different question from the
            // one the setting answers. It is the question somebody is asking
            // when they change the setting and wonder what became of their
            // history: a later change to a named language leaves the older half
            // stemmed for search in a language nothing now says.
            //
            // Information and not a fault. A store in two languages works, and
            // `--repair` deliberately does not re-stem it while a language is
            // named: the row was recorded under a language somebody chose, and
            // there is no other answer this check could name.
            //
            // With no language set the question is a different one: each row
            // is recorded under the language its words are in, so a row that is
            // not is one the triggers did not write that way (it predates
            // migration 23, or was imported), and `--repair` can say what the
            // answer is by reading it again.
            let effective = self.memory_language();
            let misrecorded = self.rows_in_another_language()?;
            let outside: Vec<(String, i64)> = if self.detects_language() {
                Vec::new()
            } else {
                self.stem_language_counts()?
                    .into_iter()
                    .filter(|(code, _)| code != effective.code())
                    .collect()
            };
            record(if misrecorded > 0 {
                DoctorCheck::noted(
                    "memory_language",
                    format!(
                        "{misrecorded} memories are recorded under a language other than the one their words are in, so search stems them wrongly; `leteo doctor --repair` re-stems them"
                    ),
                )
            } else if outside.is_empty() {
                DoctorCheck::passed("memory_language")
            } else {
                let held = outside
                    .iter()
                    .map(|(code, count)| format!("{count} in {code}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                DoctorCheck::noted(
                    "memory_language",
                    format!(
                        "a new memory is written in {}, but {held} memories already saved keep the language they were written in and are not converted",
                        effective.code()
                    ),
                )
            });
        }
        if want("semantic_model") {
            // The model semantic search reads, which lives in a file and not in
            // the database, and which three different things can be wrong with.
            // Naming which is the point: "search is by words only" is the
            // symptom of all three, and the fix differs. A missing or unverified
            // model is a warning — the store works, one capability is gone —
            // and never an error, because an optional thing that is not there is
            // not a broken store.
            record(
                if !self
                    .config
                    .database_path
                    .parent()
                    .map(crate::settings::load)
                    .unwrap_or_default()
                    .semantic_search()
                {
                    DoctorCheck::noted(
                        "semantic_model",
                        "semantic search is turned off by the semantic_search setting, so the model is not looked for",
                    )
                } else {
                    match crate::semantic::status(self.data_dir(), self.model_dir()) {
                        status @ crate::semantic::Status::Verified(_) => {
                            DoctorCheck::noted("semantic_model", status.explain())
                        }
                        status => DoctorCheck::warned("semantic_model", status.explain()),
                    }
                },
            );
        }
        if want("semantic_vectors") {
            // How much of the store the semantic stage can see.
            //
            // A warning rather than a note, like the missing model: a store with
            // no vectors searches by words alone, and this is the one place that
            // says so — a search that finds nothing by meaning looks exactly like
            // a store with nothing to find. Turned off by the setting, nothing
            // embeds and nothing reads, so the count would name a repair that
            // does nothing and it says so instead.
            record(
                if !self
                    .config
                    .database_path
                    .parent()
                    .map(crate::settings::load)
                    .unwrap_or_default()
                    .semantic_search()
                {
                    DoctorCheck::noted(
                        "semantic_vectors",
                        "semantic search is turned off by the semantic_search setting, so no vector is made or read",
                    )
                } else {
                    match self.vector_coverage() {
                        Ok((covered, total)) if covered == total => DoctorCheck::noted(
                            "semantic_vectors",
                            format!(
                                "every one of the {total} memories the stage can reach has a current vector"
                            ),
                        ),
                        Ok((covered, total)) => DoctorCheck::warned(
                            "semantic_vectors",
                            format!(
                                "{covered} of {total} memories the stage can reach have a current vector; the rest are found by their words only until the background backfill or `leteo doctor --repair` embeds them"
                            ),
                        ),
                        Err(error) => DoctorCheck::failed(
                            "semantic_vectors",
                            format!("the vector coverage could not be read: {error}"),
                        ),
                    }
                },
            );
        }
        if want("topic_key_uniqueness") {
            // One live memory per key, per project, per scope — and the one
            // operation that can break it says so once and then nothing does.
            //
            // Merging two projects can leave two memories under one
            // key, because each may have had its own, and the merge reports how
            // many rather than choosing which to keep. That report is a number
            // in one reply. After it, the store carries an ambiguity nothing
            // mentions again, and the cost is not theoretical: the next save
            // under that key revises whichever row the lookup reaches first and
            // the other can never be revised by its own key again. Driven on a
            // merged store, that is exactly what happens, and `doctor` called it
            // healthy.
            //
            // No `--repair`, deliberately. Which of the two keeps the key is a
            // question about what they say, and Leteo does not read them.
            record(match shared_topic_keys(&self.connection) {
                Err(error) => could_not_run("topic_key_uniqueness", "the topic keys", &error),
                Ok(0) => DoctorCheck::passed("topic_key_uniqueness"),
                Ok(shared) => DoctorCheck::failed(
                    "topic_key_uniqueness",
                    format!(
                        "{shared} topic key(s) name more than one live memory in the same project and scope, so a save under one of them revises whichever comes first and leaves the rest unreachable by that key; read them with `leteo search <key>` and give one of each pair a key of its own"
                    ),
                ),
            });
        }
        if want("observation_type_searchable") {
            // A memory filed under a word no filter can ask for.
            //
            // The category is a search filter, and the save door folds an
            // unknown word onto `discovery` — `normalize::kind` again — because
            // a memory nothing can filter to is a memory nothing finds. A store
            // written before that fold still holds the old words, and this is
            // where it is said. A warning rather than an error: the memories are
            // there and a full-text search still reaches them.
            //
            // `doctor --repair` folds them through the same `normalize::kind`
            // the door uses, which is the one place the rule lives.
            record(match unsearchable_kinds(&self.connection).as_deref() {
                Err(error) => {
                    could_not_run("observation_type_searchable", "the memory types", error)
                }
                Ok([]) => DoctorCheck::passed("observation_type_searchable"),
                Ok(kinds) => {
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
                    DoctorCheck::warned(
                        "observation_type_searchable",
                        format!(
                            "{total} memories carry a type no filtered search can name: {named}{and_more}. The save door now folds an unknown type onto {}; `leteo doctor --repair` folds these too",
                            crate::memory::rules::KINDS.join(", ")
                        ),
                    )
                }
            });
        }
        if want("observation_hash_sync") {
            // The hash is what dedupe compares — a save whose body matches an
            // existing one bumps that row instead of writing a second — so a
            // hash that has stopped matching its own content is a memory nothing
            // can ever be deduplicated against, silently and for good.
            //
            // It happens. A real store of 3,940 held three, all from one project
            // on one day five weeks earlier, none of them ever revised, and the
            // text their hashes were taken of is in no row of that store.
            // Whatever wrote them is gone; what was missing was anything that
            // would notice.
            record(match stale_hash_count(&self.connection) {
                Err(error) => could_not_run("observation_hash_sync", "the memory hashes", &error),
                Ok(0) => DoctorCheck::passed("observation_hash_sync"),
                Ok(stale) => DoctorCheck::failed(
                    "observation_hash_sync",
                    format!(
                        "{stale} memories carry a hash that no longer describes them, so nothing can be deduplicated against them; run `leteo doctor --repair` to take it again"
                    ),
                ),
            });
        }
        if want("orphan_versions") {
            // Version rows whose memory is gone: the text a hard delete made
            // before #332 left behind, which nothing swept. A warning and not an
            // error, because the store works; what it holds is text somebody
            // asked to have removed. A replica that has pulled a memory's
            // versions and not yet its upsert shows them here for a moment too,
            // which is why `--repair` is a choice and no open does it.
            record(match orphan_version_count(&self.connection) {
                Err(error) => could_not_run("orphan_versions", "the version history", &error),
                Ok(0) => DoctorCheck::passed("orphan_versions"),
                Ok(orphans) => DoctorCheck::warned(
                    "orphan_versions",
                    format!(
                        "{orphans} version rows belong to no memory, the earlier text of memories deleted before hard deletes removed it; `leteo doctor --repair` removes them"
                    ),
                ),
            });
        }
        if want("hook_spool") {
            // Captures a busy store could not take and no later open has
            // replayed. A busy store is not damage, and the check says so with
            // work waiting rather than with a fault, as a warning: the count,
            // the age of the oldest, and the flag that replays them.
            let spool =
                crate::hooks::spool::pending(&crate::hooks::spool::directory(self.data_dir()));
            record(match spool.entries {
                0 => DoctorCheck::passed("hook_spool"),
                entries => DoctorCheck::warned(
                    "hook_spool",
                    format!(
                        "{entries} hook capture(s) are waiting beside the store to be stored and the oldest is {} old; `leteo doctor --repair` replays them",
                        spool
                            .oldest
                            .map(crate::hooks::spool::describe_age)
                            .unwrap_or_else(|| "a moment".to_owned())
                    ),
                ),
            });
        }
        if want(DoctorCheck::STALE_CITATIONS)
            && let Some(scope) = citations
        {
            record(self.stale_citations(scope));
        }

        Ok(DoctorReport {
            healthy: !checks
                .iter()
                .any(|check| !check.ok && check.severity == DoctorSeverity::Error),
            version: crate::build_info::version().to_owned(),
            schema_version: schema_version(&self.connection)?,
            schema_supported: SCHEMA_VERSION,
            checks,
            integrity_check,
            foreign_key_violations,
            observation_fts_ok,
            prompt_fts_ok,
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

    /// How many live memories the store holds under each recorded stem
    /// language, in code order.
    ///
    /// The language is stored on `observation_stems`, one row per memory, and it
    /// is fixed when the memory is written: a later change of the
    /// memory-writing setting re-stems only the rows it touches. So this answers
    /// both halves of what such a change costs — how many memories stay behind,
    /// and under which language.
    ///
    /// A memory whose `id` is not an integer has no stems row (migration 21),
    /// which is why the sum can be short of the store's live count; those rows
    /// have no language anything recorded, so they are absent rather than
    /// counted under one.
    pub fn stem_language_counts(&self) -> Result<Vec<(String, i64)>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT s.language, COUNT(*)
               FROM observations o
               JOIN observation_stems s ON s.observation_id = o.id
              WHERE o.deleted_at IS NULL
              GROUP BY s.language
              ORDER BY s.language",
        )?;
        let counts = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<(String, i64)>, _>>()?;
        Ok(counts)
    }

    /// How many rows are recorded under a language other than the one their
    /// words are in.
    ///
    /// Zero when the setting names a language: a named language is what every
    /// row is meant to be recorded under, detection is not consulted, and a row
    /// kept from an earlier setting is reported by the check that counts
    /// languages instead. Otherwise every row is compared, deleted ones too,
    /// because [`Store::restem_to_detected_language`] repairs every row and the
    /// two must be counting the same set.
    pub fn rows_in_another_language(&self) -> Result<i64, StoreError> {
        if !self.detects_language() {
            return Ok(0);
        }
        Ok(self.connection.query_row(
            "SELECT COUNT(*)
               FROM observation_stems s
               JOIN observations o ON o.id = s.observation_id
              WHERE s.language IS NOT leteo_memory_language(o.title, o.content)",
            [],
            |row| row.get(0),
        )?)
    }

    /// Records every row under the language its words are in and stems it there.
    ///
    /// The recorded language is moved first and the stems are then taken by the
    /// path every repair of them uses, which keeps a row's language and rewrites
    /// what differs from it, so there is one place that knows how a stems row is
    /// built. A row whose language was already right is not written.
    ///
    /// Does nothing when the setting names a language, and answers zero: the
    /// registered function would then answer that language for every row, and
    /// moving all of them onto it is a different act that no one asked for.
    /// Returns how many rows changed language.
    pub fn restem_to_detected_language(&mut self) -> Result<i64, StoreError> {
        if !self.detects_language() {
            return Ok(0);
        }
        let tx = self.write_transaction()?;
        let moved = tx.execute(
            "UPDATE observation_stems SET language = detected.code
               FROM (SELECT o.id AS id, leteo_memory_language(o.title, o.content) AS code
                       FROM observations o WHERE typeof(o.id) = 'integer') AS detected
              WHERE detected.id = observation_stems.observation_id
                AND observation_stems.language IS NOT detected.code",
            [],
        )? as i64;
        crate::store::schema::restem_observations(&tx)?;
        tx.commit()?;
        Ok(moved)
    }
}

impl Store {
    /// The `stale_citations` check: memories of one project that cite a commit
    /// or a path which no longer resolves in that project's repository.
    ///
    /// A warning when it finds any, never an error. It is a statement about the
    /// world, not about the store: the memories are intact, they are merely
    /// about something that has moved, and `healthy` is a verdict on the store.
    /// Nothing is hidden or marked and nothing is written; the memory text is
    /// only read.
    ///
    /// Every way it cannot answer is also a warning, with the reason in the
    /// sentence, and none is a pass: no git, no repository, a directory whose
    /// project is not the one being checked. The last matters most. Run from
    /// another repository this would test a project's hashes against history
    /// that never held them and call nearly all of them dead.
    fn stale_citations(&self, scope: &crate::citations::Scope) -> DoctorCheck {
        let code = DoctorCheck::STALE_CITATIONS;
        let unable = |reason: String| {
            DoctorCheck::warned(
                code,
                format!("stale citations could not be checked: {reason}"),
            )
        };

        let detection = crate::project::detect_project(&scope.directory);
        if detection.project.is_empty() {
            return unable(detection.error_hint.unwrap_or_else(|| {
                format!("no project is detected for {}", scope.directory.display())
            }));
        }
        if detection.is_guess() {
            return unable(format!(
                "the project of {} is only a guess ({}), so it is not known to be the one whose memories would be read",
                scope.directory.display(),
                detection.warning.as_deref().unwrap_or_default()
            ));
        }
        let project = scope
            .project
            .as_deref()
            .map(normalize::project)
            .unwrap_or_else(|| detection.project.clone());
        if project != detection.project {
            return unable(format!(
                "{} belongs to project {:?}, not {project:?}; run it from inside the repository of {project:?}",
                scope.directory.display(),
                detection.project
            ));
        }
        let Some(root) = crate::project::repository_root(&scope.directory) else {
            return unable(format!(
                "{} is not inside a git repository",
                scope.directory.display()
            ));
        };

        let memories = match self.project_memory_text(&project) {
            Ok(memories) => memories,
            Err(error) => {
                return could_not_run(code, "the memories of the project", &error);
            }
        };
        match crate::citations::assess(&root, &memories) {
            Err(reason) => unable(reason),
            Ok(assessment) if assessment.stale.is_empty() => {
                DoctorCheck::noted(code, assessment.describe(&project))
            }
            Ok(assessment) => DoctorCheck::warned(code, assessment.describe(&project)),
        }
    }

    /// The live memories of one project at project scope, as the text a citation
    /// can be in: the title and the body. Personal-scope memories are not read,
    /// because they are not about this repository.
    fn project_memory_text(&self, project: &str) -> Result<Vec<(i64, String)>, rusqlite::Error> {
        let mut statement = self.connection.prepare(
            "SELECT id, title, content FROM observations
              WHERE deleted_at IS NULL AND project = ?1 AND scope = 'project'
              ORDER BY id",
        )?;
        let rows = statement.query_map([project], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                format!("{}\n{}", row.get::<_, String>(1)?, row.get::<_, String>(2)?),
            ))
        })?;
        rows.collect()
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
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            let mut stale = Vec::new();
            for row in rows {
                let (id, content, hash) = row?;
                let taken = normalize::normalized_hash(&content);
                if hash.as_deref() != Some(taken.as_str()) {
                    stale.push((id, taken));
                }
            }
            stale
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

    /// Removes the version rows that name no memory, and says how many.
    ///
    /// Hard deletes made before #332 removed the observation and kept its
    /// history, and nothing has swept those since. The same predicate the
    /// `orphan_versions` check counts with, so the two cannot disagree about
    /// what an orphan is.
    pub fn remove_orphan_versions(&mut self) -> Result<i64, StoreError> {
        let tx = self.write_transaction()?;
        let removed = tx.execute(
            &format!("DELETE FROM observation_versions WHERE {ORPHAN_VERSION_PREDICATE}"),
            [],
        )?;
        tx.commit()?;
        Ok(removed as i64)
    }

    /// Folds the types an adoption copied before it folded them.
    ///
    /// A store adopted by an earlier build holds Engram's words — `bug`,
    /// `manual`, `learning` — and `doctor` reports it unhealthy for exactly
    /// that: a filtered search can never return those memories. New adoptions
    /// fold as they copy; this recovers the ones already in.
    ///
    /// Only the synonyms [`normalize::kind`] maps onto a documented kind are
    /// written, so a type outside the vocabulary — `implementation`, `feature`
    /// — is left as the word somebody meant, which is what the save door does.
    /// No migration: a released one is never edited, and a new one would run
    /// once and leave every store opened before it to another path.
    pub fn fold_observation_types(&mut self) -> Result<i64, StoreError> {
        let kinds: Vec<String> = {
            let mut statement = self
                .connection
                .prepare("SELECT DISTINCT type FROM observations")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let changes: Vec<(String, String)> = kinds
            .into_iter()
            .filter_map(|kind| {
                let folded = normalize::kind(&kind);
                (folded != kind && crate::memory::rules::KINDS.contains(&folded.as_str()))
                    .then_some((kind, folded))
            })
            .collect();
        if changes.is_empty() {
            return Ok(0);
        }
        let tx = self.write_transaction()?;
        let mut moved = 0_i64;
        for (from, to) in &changes {
            moved += tx.execute(
                "UPDATE observations SET type = ?1 WHERE type = ?2",
                params![to, from],
            )? as i64;
        }
        tx.commit()?;
        Ok(moved)
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

/// How many topic keys name more than one live memory in one project and scope.
///
/// Seven milliseconds on a store of four thousand, served by `idx_obs_topic`,
/// which is the index the revision lookup already uses — the check asks the
/// same question that lookup asks, and counts the answers it should never have
/// more than one of.
fn shared_topic_keys(connection: &Connection) -> Result<i64, rusqlite::Error> {
    connection.query_row(
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
}

/// The kinds this store holds that no filtered search can name, with how many
/// memories carry each, commonest first.
///
/// Read as every distinct kind and then filtered through
/// [`crate::memory::rules::is_searchable_kind`], rather than written as a `NOT
/// IN` list in the SQL. The list of kinds is one list, and a second copy here is
/// how it would come to disagree with the door that writes them — which is the
/// mistake `REVIEW_WINDOWS` made with a third hand-written copy of three names.
fn unsearchable_kinds(connection: &Connection) -> Result<Vec<(String, i64)>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT type, COUNT(*) FROM observations WHERE deleted_at IS NULL GROUP BY type",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut found = Vec::new();
    for row in rows {
        let (kind, count) = row?;
        if !crate::memory::rules::is_searchable_kind(&kind) {
            found.push((kind, count));
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(found)
}

/// A version row no memory of this store owns. A soft-deleted memory still has
/// its row, so its history is not an orphan.
const ORPHAN_VERSION_PREDICATE: &str = "NOT EXISTS (SELECT 1 FROM observations WHERE observations.sync_id = observation_versions.observation_sync_id)";

fn orphan_version_count(connection: &Connection) -> Result<i64, rusqlite::Error> {
    connection.query_row(
        &format!("SELECT COUNT(*) FROM observation_versions WHERE {ORPHAN_VERSION_PREDICATE}"),
        [],
        |row| row.get(0),
    )
}

/// How many memories carry a hash that no longer describes their body.
///
/// Reads every live body, which is what makes it honest: the hash is derived
/// from the text and the only way to know they still agree is to take it again.
///
/// A memory with no hash at all is stale: a database adopted from an early
/// Engram schema gets the column added nullable and never filled, and reading it
/// as a non-null string dropped exactly those rows from the count.
fn stale_hash_count(connection: &Connection) -> Result<i64, rusqlite::Error> {
    let mut statement = connection
        .prepare("SELECT content, normalized_hash FROM observations WHERE deleted_at IS NULL")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let mut stale = 0;
    for row in rows {
        let (content, hash) = row?;
        if hash.as_deref() != Some(normalize::normalized_hash(&content).as_str()) {
            stale += 1;
        }
    }
    Ok(stale)
}

/// What a check reports when its query failed, so it is not read as a pass.
///
/// An error is not an answer: turning one into zero is how a store that could
/// not be read came to be reported as one with nothing wrong. The severity is an
/// error for the reason the index checks give — a store that could not be
/// inspected has not passed — and the sentence says it did not run, which is not
/// the same as having found a fault.
fn could_not_run(code: &str, what: &str, error: &dyn std::fmt::Display) -> DoctorCheck {
    DoctorCheck::failed(code, format!("{what} could not be checked: {error}"))
}
