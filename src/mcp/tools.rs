use super::*;
use rmcp::handler::server::router::tool::ToolRouter;

impl LeteoMcpServer {
    pub(super) fn router() -> ToolRouter<Self> {
        Self::tool_router()
    }
    fn read_project(&self) -> Option<(String, String)> {
        if let Some(default_project) = &self.default_project {
            return Some((
                default_project.clone(),
                crate::project::SOURCE_PROCESS_OVERRIDE.to_owned(),
            ));
        }
        let detection = detect_current_project();
        (detection.error_hint.is_none() && !detection.project.is_empty())
            .then_some((detection.project, detection.source))
    }
}

#[tool_router]
impl LeteoMcpServer {
    #[tool(
        name = "mem_save",
        // The judgment sentence is here and not only in the server's
        // `instructions`, which is where it used to live alone. A client that
        // does not surface that block hands the agent a reply with `candidates`
        // in it and nothing that says they have to be settled — and an
        // unjudged pair is dropped, not deferred, so the cost of not knowing is
        // silent. Continued with `\` rather than `\n`, for the reason written
        description = "Save an observation to persistent memory — a decision, a fix, a \
                       discovery, a convention — as it happens rather than at the end. \
                       Without session_id, uses a stable manual-save session for the \
                       detected project. A reply carrying `candidates` means this may \
                       contradict memories already held: settle each one with mem_judge in \
                       the same turn, because a pair left unjudged is dropped rather than \
                       deferred.",
        annotations(
            title = "Save Memory",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_save(
        &self,
        Parameters(params): Parameters<SaveParams>,
    ) -> Result<Json<SaveOutput>, CallToolResult> {
        let content = params
            .content
            .or(params.observation)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                structured_error(
                    error_code::INVALID_PARAMS,
                    "content is required (use content, or observation for compatible clients)",
                )
            })?;
        let mut store = self.lock_store()?;
        let context = self.write_session(
            &mut store,
            params.session_id,
            params.project,
            ProjectChoice {
                reason: params.project_choice_reason,
                recovery_token: params.recovery_token,
            },
        )?;
        let asked_scope = Some(params.scope.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        let prompt_sync_id = params.capture_prompt.then(|| {
            self.prompt_context
                .lock()
                .ok()
                .and_then(|current| {
                    current
                        .as_ref()
                        .filter(|prompt| prompt.matches(&context.project, &context.id))
                        .map(|prompt| prompt.sync_id.clone())
                })
                // Then the store, because the prompt usually was not recorded
                // here. `mem_save_prompt` sets the context above, but prompts
                // are captured by the `user-prompt-submit` hook — a separate
                // process — so the server's copy stays `None` and this link
                // was never made. A real store of 3,550 memories carried
                // `prompt_sync_id` on none of them.
                .or_else(|| {
                    store.prompt_behind_a_save(&context.id, &context.project, context.named)
                })
        });
        let prompt_sync_id = prompt_sync_id.flatten();
        let outcome = store
            .add_observation(AddObservation {
                session_id: context.id,
                kind: params.kind,
                title: params.title,
                content,
                tool_name: params.tool_name,
                project: Some(context.project),
                scope: params.scope,
                topic_key: params.topic_key,
                prompt_sync_id,
            })
            .map_err(store_error)?;
        let candidates = if outcome.kind == AddOutcomeKind::Deduplicated {
            Vec::new()
        } else {
            store
                .find_candidates(
                    outcome.observation.id,
                    CandidateOptions {
                        project: outcome.observation.project.clone(),
                        scope: Some(outcome.observation.scope.clone()),
                        ..CandidateOptions::default()
                    },
                )
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "post-save conflict detection failed");
                    Vec::new()
                })
        };

        let unfiled = !crate::memory::rules::is_searchable_kind(&outcome.observation.kind);
        let refiled = asked_scope
            .filter(|asked| !crate::memory::normalize::SCOPES.contains(&asked.as_str()))
            .map(|asked| crate::mcp::output::refiled_scope_hint(&asked));
        let mut saved = SaveOutput::new(outcome, candidates, context.envelope);
        saved.hint = match (unfiled.then(|| UNFILED_KIND_HINT.to_owned()), refiled) {
            (Some(kind), Some(scope)) => Some(format!("{kind} {scope}")),
            (kind, scope) => kind.or(scope),
        };
        Ok(Json(saved))
    }

    #[tool(
        name = "mem_update",
        description = "Revise a stored memory. Fields left out keep their current value. The memory comes back as a 400-character preview marked `content_truncated`; read one in full with mem_get_observation.",
        annotations(
            title = "Update Memory",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_update(
        &self,
        Parameters(params): Parameters<UpdateParams>,
    ) -> Result<Json<ObservationResultOutput>, CallToolResult> {
        if params.is_empty() {
            return Err(structured_error(
                error_code::INVALID_PARAMS,
                "provide at least one field to update",
            ));
        }
        let mut store = self.lock_store()?;
        let project = match params.project {
            Some(requested) => Some(
                self.resolve_write_project(
                    &store,
                    Some(requested),
                    &detect_current_project(),
                    ProjectChoice::default(),
                )?
                .0,
            ),
            None => None,
        };
        let observation = store
            .update_observation(
                params.id,
                UpdateObservation {
                    kind: params.kind,
                    title: params.title,
                    content: params.content,
                    project,
                    scope: params.scope,
                    topic_key: params.topic_key,
                },
            )
            .map_err(store_error)?;
        let caveats = store
            .caveats_for(std::slice::from_ref(&observation.sync_id))
            .unwrap_or_default()
            .remove(&observation.sync_id)
            .unwrap_or_default();
        drop(store);

        let mut observation = ObservationOutput::from(observation).preview();
        observation.caveats = caveats.into_iter().map(Into::into).collect();
        Ok(Json(ObservationResultOutput { observation }))
    }

    #[tool(
        name = "mem_review",
        description = "List observations due for review or mark one reviewed. Actions: \
                       list, mark_reviewed. Bodies come back as a 400-character preview \
                       marked `content_truncated`; read one in full with \
                       mem_get_observation.",
        annotations(
            title = "Review Memories",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_review(
        &self,
        Parameters(params): Parameters<ReviewParams>,
    ) -> Result<Json<ReviewOutput>, CallToolResult> {
        match params.action.trim() {
            "list" => {
                let store = self.lock_store()?;
                let observations = store
                    .observations_needing_review(
                        params.project.as_deref(),
                        Some(params.limit.min(store.max_context_results())),
                    )
                    .map_err(store_error)?;
                let named: Vec<String> = observations
                    .iter()
                    .map(|observation| observation.sync_id.clone())
                    .collect();
                let caveats = store.caveats_for(&named).unwrap_or_default();
                let due = store
                    .count_review_due(params.project.as_deref())
                    .unwrap_or_default()
                    .max(0) as usize;
                Ok(Json(ReviewOutput::listing(observations, &caveats, due)))
            }
            "mark_reviewed" => {
                let id = params.observation_id.or(params.id).ok_or_else(|| {
                    structured_error(
                        error_code::INVALID_PARAMS,
                        "observation_id is required for mark_reviewed",
                    )
                })?;
                let mut store = self.lock_store()?;
                store.mark_reviewed(id).map_err(store_error)?;
                let observation = store.get_observation(id).map_err(store_error)?;
                Ok(Json(ReviewOutput::marked(observation)))
            }
            _ => Err(structured_error(
                error_code::INVALID_PARAMS,
                "action must be one of: list, mark_reviewed",
            )),
        }
    }

    #[tool(
        name = "mem_suggest_topic_key",
        description = "Suggest a stable topic_key for observation upserts.",
        annotations(
            title = "Suggest Topic Key",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_suggest_topic_key(
        &self,
        Parameters(params): Parameters<SuggestTopicKeyParams>,
    ) -> Result<Json<SuggestTopicKeyOutput>, CallToolResult> {
        if params.title.trim().is_empty() && params.content.trim().is_empty() {
            return Err(structured_error(
                error_code::INVALID_PARAMS,
                "a topic_key can only be suggested from a title or some content",
            ));
        }
        Ok(Json(SuggestTopicKeyOutput {
            topic_key: suggest_topic_key(&params.kind, &params.title, &params.content),
        }))
    }

    #[tool(
        name = "mem_delete",
        description = "Delete an observation by ID. Soft-delete by default; hard_delete permanently removes it.",
        annotations(
            title = "Delete Memory",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_delete(
        &self,
        Parameters(params): Parameters<DeleteParams>,
    ) -> Result<Json<DeleteOutput>, CallToolResult> {
        self.lock_store()?
            .delete_observation(params.id, params.hard_delete)
            .map_err(store_error)?;
        Ok(Json(DeleteOutput {
            id: params.id,
            hard_delete: params.hard_delete,
            status: if params.hard_delete {
                "deleted"
            } else {
                "soft_deleted"
            }
            .to_owned(),
        }))
    }

    #[tool(
        name = "mem_search",
        description = "Search persistent observations by full-text query and optional \
                       filters. Answers about the current project unless you pass a \
                       project or all_projects. Long bodies come back as a 400-character \
                       preview marked `content_truncated`; read one in full with \
                       mem_get_observation.",
        annotations(
            title = "Search Memory",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> Result<Json<SearchOutput>, CallToolResult> {
        let fallback = (!params.all_projects)
            .then(|| self.read_project())
            .flatten();
        let project = if params.all_projects {
            None
        } else {
            params
                .project
                .clone()
                .or_else(|| fallback.as_ref().map(|(project, _)| project.clone()))
        };
        let envelope = if params.all_projects {
            ProjectEnvelope {
                project_source: SOURCE_ALL_PROJECTS.to_owned(),
                ..ProjectEnvelope::default()
            }
        } else {
            ProjectEnvelope::for_read(params.project.as_deref(), fallback.as_ref())
        };
        let store = self.lock_store()?;
        let cap = store.max_search_results();
        let kind = params.kind.clone();
        let scope = params.scope.clone();
        let limit = params.limit;
        let mode = params.match_mode.into();
        let (results, more) = store
            .search_with_more(
                &params.query,
                SearchOptions {
                    kind: params.kind,
                    project,
                    scope: params.scope,
                    limit: params.limit,
                    mode,
                },
            )
            .map_err(store_error)?;
        let clamped = more && results.len() >= cap;
        let named: Vec<String> = results
            .iter()
            .map(|result| result.observation.sync_id.clone())
            .collect();
        let caveats = store.caveats_for(&named).unwrap_or_default();
        let inferred = params.project.is_none() && !params.all_projects;
        let elsewhere = match (results.is_empty() && inferred, fallback.as_ref()) {
            (true, Some((project, _))) => store
                .search(
                    &params.query,
                    SearchOptions {
                        project: None,
                        kind: kind.clone(),
                        scope: scope.clone(),
                        limit,
                        mode,
                    },
                )
                .ok()
                .map(|found| {
                    (
                        project.clone(),
                        found.len(),
                        limit.unwrap_or(crate::store::DEFAULT_SEARCH_LIMIT),
                    )
                }),
            _ => None,
        };

        Ok(Json(SearchOutput::new(
            results, envelope, clamped, &caveats, elsewhere, more,
        )))
    }

    #[tool(
        name = "mem_get_observation",
        description = "Get one complete observation by its numeric identifier, with its \
                       full body — unlike mem_search and mem_context, which preview it. \
                       Reads `state`: a memory that has been deleted is still returned \
                       here and says so.",
        annotations(
            title = "Get Observation",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_get_observation(
        &self,
        Parameters(params): Parameters<GetObservationParams>,
    ) -> Result<Json<ObservationResultOutput>, CallToolResult> {
        let store = self.lock_store()?;
        let observation = store.get_observation(params.id).map_err(store_error)?;
        let caveats = store
            .caveats_for(std::slice::from_ref(&observation.sync_id))
            .unwrap_or_default()
            .remove(&observation.sync_id)
            .unwrap_or_default();
        drop(store);

        let mut observation = ObservationOutput::from(observation);
        observation.caveats = caveats.into_iter().map(Into::into).collect();
        Ok(Json(ObservationResultOutput { observation }))
    }

    #[tool(
        name = "mem_context",
        description = "Get pinned and recent observations plus the recent sessions and \
                       user prompts of a project. Answers about the current project \
                       unless you pass a project or all_projects. Long bodies come back \
                       as a 400-character preview marked `content_truncated`; read one \
                       in full with mem_get_observation.",
        annotations(
            title = "Get Memory Context",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_context(
        &self,
        Parameters(params): Parameters<ContextParams>,
    ) -> Result<Json<ContextOutput>, CallToolResult> {
        let fallback = (!params.all_projects)
            .then(|| self.read_project())
            .flatten();
        let envelope = if params.all_projects {
            ProjectEnvelope {
                project_source: SOURCE_ALL_PROJECTS.to_owned(),
                ..ProjectEnvelope::default()
            }
        } else {
            ProjectEnvelope::for_read(params.project.as_deref(), fallback.as_ref())
        };
        let inferred = params.project.is_none() && !params.all_projects;
        let project = if params.all_projects {
            None
        } else {
            params
                .project
                .or_else(|| fallback.as_ref().map(|(project, _)| project.clone()))
        };
        let store = self.lock_store()?;
        // One read of the settings file for the two answers that come out of
        // it, rather than one each. Read on each call rather than captured at
        // start-up — somebody who changes the size or the language is answered
        // by the next call, not by restarting their client — but it was read
        // twice per call, once here and once for the language forty lines down,
        // with the same sentence justifying both. At 7.5 microseconds a read
        // that is nothing against the 830 this call costs; the reason to do it
        // once is that two reads of one file can disagree.
        let settings = crate::settings::load_beside(store.database_path());
        let limit = params
            .limit
            .unwrap_or_else(|| settings.context_size().memories())
            .min(crate::settings::ContextSize::Deep.memories());
        let list_ceiling = store.max_context_results();
        let session_limit = params.session_limit.min(list_ceiling);
        let prompt_limit = params.prompt_limit.min(list_ceiling);
        let (mut observations, pinned_omitted) = store
            .pinned_observations(project.as_deref(), params.scope.as_deref(), limit)
            .map_err(store_error)?;
        let pinned = observations.len();
        let mut sessions = store
            .recent_sessions(project.as_deref(), Some(session_limit))
            .map_err(store_error)?;
        let mut recent: Vec<_> = store
            .recent_memories(project.as_deref(), params.scope.as_deref(), limit)
            .map_err(store_error)?
            .into_iter()
            .filter(|observation| !observations.iter().any(|item| item.id == observation.id))
            .collect();
        let summaries = store
            .session_summaries(
                &sessions
                    .iter()
                    .map(|session| session.id.clone())
                    .collect::<Vec<_>>(),
            )
            .map_err(store_error)?;
        crate::recall::fold_session_summaries(&mut sessions, summaries);
        observations.append(&mut recent);
        observations.truncate(pinned.saturating_add(limit));
        let prompts = store
            .recent_distinct_prompts(project.as_deref(), Some(prompt_limit))
            .map_err(store_error)?;

        let language = settings.language_directive();
        let named: Vec<String> = observations
            .iter()
            .map(|observation| observation.sync_id.clone())
            .collect();
        let caveats = store.caveats_for(&named).unwrap_or_default();
        let elsewhere = match (
            observations.is_empty() && sessions.is_empty() && prompts.is_empty() && inferred,
            project.as_deref(),
        ) {
            (true, Some(project)) => store
                .memories_outside(project, crate::mcp::ELSEWHERE_CAP)
                .ok()
                .filter(|held| *held > 0)
                .map(|held| (project.to_owned(), held as usize)),
            _ => None,
        };
        Ok(Json(ContextOutput::new(
            observations,
            pinned,
            pinned_omitted,
            sessions,
            prompts,
            crate::mcp::output::ContextEnvelope {
                project: envelope,
                memory_language: language,
                elsewhere,
            },
            &caveats,
        )))
    }

    #[tool(
        name = "mem_save_prompt",
        description = "Save a user prompt in an existing session. The prompt comes back as a 400-character preview marked `content_truncated`; keep the sync_id to link a later save to it.",
        annotations(
            title = "Save User Prompt",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_save_prompt(
        &self,
        Parameters(params): Parameters<SavePromptParams>,
    ) -> Result<Json<PromptResultOutput>, CallToolResult> {
        let mut store = self.lock_store()?;
        let context = self.write_session(
            &mut store,
            params.session_id,
            params.project,
            ProjectChoice {
                reason: params.project_choice_reason,
                recovery_token: params.recovery_token,
            },
        )?;
        let prompt = store
            .add_prompt(AddPrompt {
                session_id: context.id.clone(),
                content: params.content,
                project: Some(context.project.clone()),
            })
            .map_err(store_error)?;

        if let Ok(mut current) = self.prompt_context.lock() {
            *current = Some(PromptContext {
                sync_id: prompt.sync_id.clone(),
                project: context.project,
                session_id: context.id,
            });
        }

        Ok(Json(PromptResultOutput {
            project_context: context.envelope,
            prompt: prompt.into(),
        }))
    }

    #[tool(
        name = "mem_session_start",
        description = "Create a memory session, or return it unchanged if its identifier exists.",
        annotations(
            title = "Start Session",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_session_start(
        &self,
        Parameters(params): Parameters<SessionStartParams>,
    ) -> Result<Json<SessionResultOutput>, CallToolResult> {
        let requested_directory = params.directory.filter(|value| !value.trim().is_empty());
        let detection = requested_directory
            .as_deref()
            .map_or_else(detect_current_project, detect_project);
        let project = resolve_detected_project(
            params.project.or_else(|| self.default_project.clone()),
            &detection,
        )?;
        let directory = requested_directory.unwrap_or(detection.path);
        let session = self
            .lock_store()?
            .create_session(&params.id, &project, &directory)
            .map_err(store_error)?;

        Ok(Json(SessionResultOutput {
            session: session.into(),
        }))
    }

    #[tool(
        name = "mem_session_end",
        description = "End an existing memory session and optionally attach a summary. The session comes back with its summary as a 400-character preview marked `summary_truncated`.",
        annotations(
            title = "End Session",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_session_end(
        &self,
        Parameters(params): Parameters<SessionEndParams>,
    ) -> Result<Json<SessionResultOutput>, CallToolResult> {
        let session = self
            .lock_store()?
            .end_session(&params.id, params.summary.as_deref())
            .map_err(store_error)?;

        Ok(Json(SessionResultOutput {
            session: session.into(),
        }))
    }

    #[tool(
        name = "mem_pin",
        description = "Pin a local observation so it appears before recent observations in memory context.",
        annotations(
            title = "Pin Memory",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_pin(
        &self,
        Parameters(params): Parameters<PinParams>,
    ) -> Result<Json<PinOutput>, CallToolResult> {
        self.set_pin(params.id, true)
    }

    #[tool(
        name = "mem_unpin",
        description = "Unpin a local observation so it returns to normal recency order.",
        annotations(
            title = "Unpin Memory",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_unpin(
        &self,
        Parameters(params): Parameters<PinParams>,
    ) -> Result<Json<PinOutput>, CallToolResult> {
        self.set_pin(params.id, false)
    }

    #[tool(
        name = "mem_timeline",
        description = "Show chronological context around a specific observation. Bodies \
                       come back as a 400-character preview marked `content_truncated`; \
                       read one in full with mem_get_observation.",
        annotations(
            title = "Memory Timeline",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_timeline(
        &self,
        Parameters(params): Parameters<TimelineParams>,
    ) -> Result<Json<TimelineOutput>, CallToolResult> {
        let timeline = self
            .lock_store()?
            .timeline(
                params.observation_id,
                Some(params.before),
                Some(params.after),
            )
            .map_err(store_error)?;
        if let Some(project) = params.project {
            let project = normalize::project(&project);
            if timeline.focus.project.as_deref() != Some(project.as_str()) {
                return Err(structured_error(
                    error_code::PROJECT_MISMATCH,
                    format!(
                        "observation {} does not belong to project {project}",
                        params.observation_id
                    ),
                ));
            }
        }
        let named: Vec<String> = std::iter::once(timeline.focus.sync_id.clone()).collect();
        let caveats = self
            .lock_store()?
            .caveats_for(&named)
            .unwrap_or_default()
            .remove(&timeline.focus.sync_id)
            .unwrap_or_default();
        let mut output: TimelineOutput = timeline.into();
        output.focus.caveats = caveats.into_iter().map(Into::into).collect();
        Ok(Json(output))
    }

    #[tool(
        name = "mem_session_summary",
        description = "Save a structured end-of-session summary as persistent memory.",
        annotations(
            title = "Save Session Summary",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_session_summary(
        &self,
        Parameters(params): Parameters<SessionSummaryParams>,
    ) -> Result<Json<SaveOutput>, CallToolResult> {
        if params.content.trim().is_empty() {
            return Err(structured_error(
                error_code::INVALID_PARAMS,
                "a session summary needs content",
            ));
        }
        let mut store = self.lock_store()?;
        let context = self.write_session(
            &mut store,
            params.session_id,
            params.project,
            ProjectChoice {
                reason: params.project_choice_reason,
                recovery_token: params.recovery_token,
            },
        )?;
        let headline = crate::memory::normalize::headline(
            &params.content,
            crate::store::SUMMARY_HEADLINE_CHARS,
        );
        let outcome = store
            .add_observation(AddObservation {
                session_id: context.id,
                kind: "session_summary".to_owned(),
                title: headline
                    .clone()
                    .unwrap_or_else(|| format!("Session summary: {}", context.project)),
                content: params.content,
                tool_name: None,
                project: Some(context.project),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .map_err(store_error)?;
        let mut saved = SaveOutput::new(outcome, Vec::new(), context.envelope);
        if headline.is_none() {
            saved.hint = Some(UNNAMED_SUMMARY_HINT.to_owned());
        }
        Ok(Json(saved))
    }

    #[tool(
        name = "mem_capture_passive",
        description = "Extract and save the Key Learnings items a subagent ended with, in any of the twelve languages Leteo writes memories in. Each becomes a memory of its own, filed under the tool that produced it.",
        annotations(
            title = "Capture Learnings",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_capture_passive(
        &self,
        Parameters(params): Parameters<CapturePassiveParams>,
    ) -> Result<Json<CapturePassiveOutput>, CallToolResult> {
        if params.content.trim().is_empty() {
            return Err(structured_error(
                error_code::INVALID_PARAMS,
                "content is required for mem_capture_passive",
            ));
        }
        let mut store = self.lock_store()?;
        let context = self.write_session(
            &mut store,
            params.session_id,
            None,
            ProjectChoice::default(),
        )?;
        let source = if params.source.trim().is_empty() {
            default_passive_source()
        } else {
            params.source
        };
        let result = store
            .passive_capture(PassiveCapture {
                session_id: context.id,
                content: params.content,
                project: context.project,
                source,
            })
            .map_err(store_error)?;
        Ok(Json(CapturePassiveOutput::new(result, context.envelope)))
    }

    #[tool(
        name = "mem_merge_projects",
        description = "Merge comma-separated project name variants into one canonical project.",
        annotations(
            title = "Merge Projects",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_merge_projects(
        &self,
        Parameters(params): Parameters<MergeProjectsParams>,
    ) -> Result<Json<MergeProjectsOutput>, CallToolResult> {
        let sources = params
            .from
            .split(',')
            .map(str::trim)
            .filter(|source| !source.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if sources.is_empty() || params.to.trim().is_empty() {
            return Err(structured_error(
                error_code::INVALID_PARAMS,
                "both 'from' and 'to' are required",
            ));
        }
        let result = self
            .lock_store()?
            .merge_projects(&sources, &params.to)
            .map_err(store_error)?;
        Ok(Json(result.into()))
    }

    #[tool(
        name = "mem_current_project",
        description = "Detect the current project without failing on ambiguous or invalid project context.",
        annotations(
            title = "Detect Current Project",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_current_project(
        &self,
        _: Parameters<NoParams>,
    ) -> Result<Json<CurrentProjectOutput>, CallToolResult> {
        let mut detection = detect_current_project();
        if let Some(default_project) = &self.default_project {
            detection.project = default_project.clone();
            detection.source = crate::project::SOURCE_PROCESS_OVERRIDE.to_owned();
            detection.error_hint = None;
        }
        Ok(Json(detection.into()))
    }

    #[tool(
        name = "mem_doctor",
        description = "Run read-only SQLite, FTS, foreign-key, and mutation-journal diagnostics.",
        annotations(
            title = "Memory Diagnostics",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_doctor(
        &self,
        Parameters(params): Parameters<DoctorParams>,
    ) -> Result<Json<DoctorOutput>, CallToolResult> {
        let requested = params.project.clone();
        let store = self.lock_store()?;
        let (report, stats) = store
            .doctor_scoped(params.check.as_deref(), requested.as_deref())
            .map_err(store_error)?;
        let project = stats.as_ref().map(|stats| stats.name.clone()).or_else(|| {
            let detected = detect_current_project();
            (!detected.project.is_empty()).then_some(detected.project)
        });
        Ok(Json(DoctorOutput::new(
            report,
            project,
            params.check,
            stats,
        )))
    }

    #[tool(
        name = "mem_stats",
        description = "Get aggregate memory store statistics. Takes no arguments and counts the whole store; for one project's counts call mem_doctor with that project.",
        annotations(
            title = "Memory Stats",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_stats(
        &self,
        _: Parameters<NoParams>,
    ) -> Result<Json<StatsOutput>, CallToolResult> {
        let stats = self.lock_store()?.stats().map_err(store_error)?;
        Ok(Json(stats.into()))
    }

    #[tool(
        name = "mem_judge",
        description = "Record a manual verdict on a pending relation surfaced by mem_save. Manual not_conflict verdicts are persisted. Reason and evidence each come back as a 400-character preview marked `reason_truncated` or `evidence_truncated`.",
        annotations(
            title = "Judge Conflict",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_judge(
        &self,
        Parameters(params): Parameters<JudgeParams>,
    ) -> Result<Json<JudgeOutput>, CallToolResult> {
        if params.judgment_id.trim().is_empty() {
            return Err(structured_error(
                error_code::INVALID_PARAMS,
                "judgment_id is required",
            ));
        }
        if params.relation.trim().is_empty() {
            return Err(structured_error(
                error_code::INVALID_PARAMS,
                "relation is required",
            ));
        }
        let relation = self
            .lock_store()?
            .judge_relation(JudgeRelationParams {
                judgment_id: params.judgment_id,
                relation: params.relation,
                reason: nonempty(params.reason),
                evidence: nonempty(params.evidence),
                confidence: params.confidence,
                marked_by_actor: "agent".to_owned(),
                marked_by_kind: "agent".to_owned(),
                marked_by_model: None,
                session_id: nonempty(params.session_id),
            })
            .map_err(store_error)?;
        Ok(Json(JudgeOutput {
            relation: relation.into(),
        }))
    }

    #[tool(
        name = "mem_compare",
        description = "Persist a semantic verdict between two observation IDs. Semantic not_conflict is a successful no-op.",
        annotations(
            title = "Compare Memories",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) fn mem_compare(
        &self,
        Parameters(params): Parameters<CompareParams>,
    ) -> Result<Json<CompareOutput>, CallToolResult> {
        if params.memory_id_a <= 0 || params.memory_id_b <= 0 {
            return Err(structured_error(
                error_code::INVALID_PARAMS,
                "memory_id_a and memory_id_b must be positive observation IDs",
            ));
        }
        if params.relation.trim().is_empty() {
            return Err(structured_error(
                error_code::INVALID_PARAMS,
                "relation is required",
            ));
        }
        let mut store = self.lock_store()?;
        let observation_a = store
            .get_observation(params.memory_id_a)
            .map_err(store_error)?;
        let observation_b = store
            .get_observation(params.memory_id_b)
            .map_err(store_error)?;
        let sync_id = store
            .judge_by_semantic(JudgeBySemanticParams {
                source_id: observation_a.sync_id,
                target_id: observation_b.sync_id,
                relation: params.relation,
                confidence: params.confidence,
                reasoning: params.reasoning.filter(|value| !value.trim().is_empty()),
                model: nonempty(params.model),
            })
            .map_err(store_error)?;
        Ok(Json(CompareOutput { sync_id }))
    }
}
