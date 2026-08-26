//! Every tool an agent can call, and what each one does.

use super::*;
use rmcp::handler::server::router::tool::ToolRouter;

impl LeteoMcpServer {
    /// The router the `#[tool_router]` macro builds from the methods below.
    ///
    /// The macro generates `tool_router()` private to the module it expands in,
    /// so once the tools moved into their own file nothing outside could reach
    /// it. This is that one line of reach, rather than a reason to keep eight
    /// hundred lines where they were.
    pub(super) fn router() -> ToolRouter<Self> {
        Self::tool_router()
    }
    /// The project a read narrows to when the caller named none.
    ///
    /// Writes have detected this from the start — a save lands in the project
    /// of the directory the server was started in — and reads never did: with
    /// no `--project` on the command line, which is how every installation
    /// launches it, `default_project` is `None` and every search and every
    /// context answered from every project at once. `all_projects` existed to
    /// widen a search that was already as wide as it goes.
    ///
    /// What that cost is dilution, and it is worst where two projects share a
    /// vocabulary: asking 150 real questions of `ledgerly` returned another
    /// project's memory in the top three 18.7% of the time, and pushed one of
    /// its own out of the answer 8% of the time. `mem_context` was worse than
    /// that, because it is not a question at all — it lists what is most
    /// recent, so an agent asking for its project's context got whichever
    /// project had been touched last.
    ///
    /// Only a detection with nothing wrong with it counts. An ambiguous
    /// directory or one that resolves to nothing falls back to every project,
    /// which is where a read that cannot be narrowed belongs — and is what the
    /// tool did before.
    ///
    /// `detect_current_project` caches, so this costs one walk per process.
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
        // above `mem_search`.
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
        // Kept before anything folds it, because the fold is what this has to
        // report: `normalize::scope` answers `project` for a word it does not
        // know and the caller's own value is gone by the time the memory comes
        // back.
        let asked_scope = Some(params.scope.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        // Best-effort: a memory is still worth saving when the prompt behind it
        // is unknown, so nothing here can fail the save.
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
                //
                // The two store-side rules — the session's last question, and
                // the project's for a save that named no session — live in
                // `Store::prompt_behind_a_save`, because `leteo save` writes to
                // the same table and used to record no question at all.
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
        // Asked when a memory arrives, not when a save call is made.
        //
        // A duplicate is the same memory a second time: the store folded it
        // into the row it already had, and whatever that row might contradict
        // was asked about when it was written. Asking again is not harmless
        // now that a settled pair is skipped — the search reaches past it to
        // the next candidates instead, so a memory saved ten times would file
        // thirty questions, each one worse than the last. Measured against a
        // copy of a real store before this: two saves of one memory, six
        // pending relations from one source.
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
        // Both, when both. Two mistakes in one call are two things to fix, and
        // a reply that mentions the first and swallows the second sends
        // somebody back for a second round.
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
        // A move between projects goes through the door a save goes through.
        //
        // `mem_save` refuses a project this store has never heard of, with the
        // name of the one the directory resolves to and a list of what exists;
        // `mem_update` took the same string and wrote it. So the guard held
        // for creating a memory and not for moving one, and the memory landed
        // in a project nothing else agreed existed — which is not a cosmetic
        // difference, because every read narrows by project: the memory stayed
        // in the store, out of every search, every opening context and every
        // hint, findable by nobody.
        //
        // Only when a project is actually named. An update that does not
        // mention the field leaves it alone, and asking the door about a move
        // nobody requested would refuse edits to memories that live in another
        // project for good reasons.
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
        // Carried here as well so the field means one thing wherever it
        // appears: what the graph says about this memory, right now. Somebody
        // revising a memory that a later one has already overturned is worth
        // telling at the moment they revise it.
        let caveats = store
            .caveats_for(std::slice::from_ref(&observation.sync_id))
            .unwrap_or_default()
            .remove(&observation.sync_id)
            .unwrap_or_default();
        drop(store);

        // Previewed, for the reason `mem_save` previews: the caller wrote this
        // text a moment ago and echoing it whole bills for it twice.
        //
        // It was the one write that did not. Measured on a real store, updating
        // nothing but the title of a memory with a 4,000-byte body sent back
        // 4,556 bytes — byte for byte what `mem_get_observation` sends, whose
        // whole purpose is to send the body in full — where the same save sent
        // 1,749.
        //
        // A revision can also touch fields the caller did not write, so silence
        // is not the answer either: the id and the preview are, and the tool
        // that hands a body over in full is one call away.
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
                        // The ceiling this list was missing. See `ReviewParams`.
                        Some(params.limit.min(store.max_context_results())),
                    )
                    .map_err(store_error)?;
                // What the graph says about a memory this queue is about to ask
                // somebody to reread.
                //
                // This is the strongest case there is for a caveat and it was
                // the one route without one. The queue exists to say "a
                // decision may have gone stale, read it again" — and when a
                // later memory has already overturned it, the answer to that
                // question is written down and the tool was asking for it to be
                // worked out afresh.
                let named: Vec<String> = observations
                    .iter()
                    .map(|observation| observation.sync_id.clone())
                    .collect();
                let caveats = store.caveats_for(&named).unwrap_or_default();
                // The whole queue, not this page of it. The session opening
                // names this number and sends the agent here; without it the
                // tool answers with a smaller one and nothing says which is
                // which. It is the same count the opening block reads, from the
                // same function, so the two cannot disagree.
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
        // Continued with `\` rather than with an escaped newline. `\n` puts a
        // real line break in the text an agent reads, and the source
        // indentation that follows it goes in too — these three carried a
        // hundred and fifteen characters of it between them.
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
        // An explicit all_projects search is a deliberate widening, so the
        // envelope reports it instead of silently falling back to the override.
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
        // Kept rather than moved: the same narrowings are needed a second time
        // if the answer comes back empty, and a retry that quietly dropped the
        // type filter would count memories the first question never asked for.
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
        // Whether the store's own maximum is what ended the list.
        //
        // The store clamps `limit` to its own maximum, and the parameter says
        // so — but the *reply* did not, and a clamped answer is the same shape
        // as an exhausted one. An agent that asked for fifty and got twenty had
        // no way to tell "that is everything" from "there is more, ask
        // differently", so the twenty read as the whole truth.
        //
        // Asked in terms of what was found rather than what was requested,
        // because the request answers neither half of the question. Asking for
        // fifty and matching exactly twenty used to be called clamped, and the
        // sentence it produced — "not everything that matched" — was false;
        // asking for exactly twenty on a store full of matches was called
        // nothing at all, and that is the case the sentence exists for. `more`
        // now survives the cap, so this is true by construction.
        let clamped = more && results.len() >= cap;
        // What the graph says about what is being handed back.
        //
        // The same warning the session context and `mem_context` carry. A
        // superseded decision reads exactly like one that still holds, and this
        // was the last of the three routes to hand one over in silence — and
        // the most used of them.
        //
        // A failure costs the annotation, not the search, for the same reason
        // as in the other two: a store that cannot answer about relations can
        // still answer about memories.
        let named: Vec<String> = results
            .iter()
            .map(|result| result.observation.sync_id.clone())
            .collect();
        let caveats = store.caveats_for(&named).unwrap_or_default();
        // Why the answer is empty, when the directory is what narrowed it.
        //
        // The words may have matched perfectly well, in a project this is not,
        // and `NO_MATCH_HINT` would tell an agent to rewrite a question that
        // was already right. So the store is asked once more without the
        // narrowing, and only where that finds something does the reason
        // change. `leteo search` has answered this way since the CLI reads
        // were scoped; the tool nine clients out of twelve actually use did
        // not.
        //
        // Paid only on an empty answer, and only when nobody named a project:
        // an explicit `project` is a caller who knows where they are looking,
        // and `all_projects` has already looked everywhere.
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
                // The limit travels with the count: it is the ceiling the
                // number can reach, and without it the reply claimed the page
                // size as a total.
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
        // Losing the caveats costs an annotation; losing the memory costs the
        // answer. This is the one place the whole thing is being read, so the
        // memory goes back either way.
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
        // Whether the directory chose the project, kept before the field is
        // consumed: an empty answer means something different when the caller
        // named where to look.
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
        // No floor, for the reason `mem_timeline` has none: every budget in this
        // reply is a section of it, and zero says to leave that section out.
        // Pinned memories are listed on top of this one, so zero here is "the
        // pinned ones and nothing else", which is a thing to ask for.
        // Bounded at the top as well as the bottom, which is the half this
        // surface was missing.
        //
        // `mem_timeline` was given a ceiling for exactly this reason — a window
        // of a million came back with a whole session, 191 KB — and this is the
        // tool nine of the thirteen clients have as their only route to context.
        // Its three budgets were all open at the top: asked for 9,999 of each,
        // a real store answered with 1,201 memories, 212 sessions and 120
        // prompts, in one reply of 469 KB. A payload that pushes the useful
        // part out of a context window has failed at the one thing this tool
        // is for.
        //
        // Each ceiling has one source. The memories are bounded by the deepest
        // context Leteo itself is ever configured to open with, because asking
        // for more than `--context deep` gives is asking for something no
        // installation produces. Sessions and prompts have no such setting, so
        // they take the store's own ceiling for a context read, the same one
        // `mem_timeline` uses.
        let limit = params
            .limit
            .unwrap_or_else(|| settings.context_size().memories())
            .min(crate::settings::ContextSize::Deep.memories());
        let list_ceiling = store.max_context_results();
        let session_limit = params.session_limit.min(list_ceiling);
        let prompt_limit = params.prompt_limit.min(list_ceiling);
        // A ceiling of its own, the same one the recent memories take: two
        // bounded lists, neither starving the other. See `pinned_observations`
        // and `recall::assemble_counted`, where the same sentence was equally
        // untrue: this said `ContextSize::Deep` while the budget beside it is
        // whatever the caller asked for, so the two matched only at `deep`.
        // Asked for five against a store with a hundred pins, this answered
        // with eighty-five memories and 73 KB.
        let (mut observations, pinned_omitted) = store
            .pinned_observations(project.as_deref(), params.scope.as_deref(), limit)
            .map_err(store_error)?;
        // The budget governs the recent ones. Pinned memories are listed on top
        // of it, which is the rule `recall::assemble_counted` already follows.
        //
        // Counting them against it instead meant a project with as many pins as
        // the budget got its pins and nothing else — no recent work at all,
        // from a tool whose description promises "pinned and recent". Pinning
        // is a deliberate act, and the reward for deciding what matters was to
        // stop being told what had happened.
        let pinned = observations.len();
        let mut sessions = store
            .recent_sessions(project.as_deref(), Some(session_limit))
            .map_err(store_error)?;
        // The same two questions `recall::assemble_counted` asks, asked the same
        // way. This is the other surface that hands context to an agent — the
        // skill names it as how to recover context mid-session — and every time
        // the rule was written twice, one of the copies was the worse of the
        // two: this one used to fetch exactly `limit` and then pay for the scope
        // filter and the session summaries out of the answer.
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
        // What the graph says about the memories about to be handed over.
        //
        // The hook's context has carried this since a superseded decision was
        // found being presented as current; this tool is the same handover for
        // the nine clients of twelve that run no hooks, and it was the one
        // route left without it.
        //
        // Not fatal, for the same reason as there: losing the annotation costs
        // a warning, and failing the call would cost the whole context.
        let named: Vec<String> = observations
            .iter()
            .map(|observation| observation.sync_id.clone())
            .collect();
        let caveats = store.caveats_for(&named).unwrap_or_default();
        // A context with nothing in it says whether the store is empty or the
        // directory is.
        //
        // Every instruction file Leteo writes tells the agent to call this
        // before acting, and for the nine clients of twelve that run no hooks
        // it is the first thing they read. Coming back silent and empty reads
        // as "there is no memory here", and the agent works blind past a store
        // that holds thousands one project over — which is what a directory
        // resolving somewhere quiet looks like from the inside.
        //
        // Counted, not searched: the question is whether there is anything at
        // all. Paid only when this project answered with nothing and nobody
        // named a project.
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

        // A save made later in this process, for this same work, can now say
        // which request it came from.
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
        // Session creation is the sanctioned way to introduce a project, so an
        // explicit name is accepted here without recovery-token guarding.
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
        // What the graph says about the memory the caller named.
        //
        // The focus arrives whole, which is the same read `mem_get_observation`
        // makes and the same reason a caveat belongs on it: this is the moment
        // somebody is deciding what a memory means, and a decision a later one
        // overturned looks exactly like one that still stands. Its neighbours
        // are a listing, and the listings carry it too.
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
        // Taken before the save, because the answer decides both the title and
        // whether the agent is told it wrote one nobody can find.
        let headline = crate::memory::normalize::headline(
            &params.content,
            crate::store::SUMMARY_HEADLINE_CHARS,
        );
        let outcome = store
            .add_observation(AddObservation {
                session_id: context.id,
                kind: "session_summary".to_owned(),
                // What the session was for, and nothing else.
                //
                // Every summary used to be called `Session summary: <project>`
                // and nothing more, so on a busy project hundreds of memories
                // shared a name and none could be found by it. The fix put the
                // session's own headline after that prefix, and the prefix
                // stayed — on a real store, in front of all 899 of them.
                //
                // A title is weighted five times in the ranking, so three words
                // repeated across a quarter of the store are three words that
                // match strongly and mean nothing. Measured on that store, with
                // and without: searching a summary by its own words is the same
                // either way — 89.0% against 89.5%, first place 59.0% both — and
                // `ledgerly summary` returns ten summaries out of ten with the
                // prefix and one of ten without it. `leteo session`, six against
                // one.
                //
                // What the prefix said is said better elsewhere: the type field
                // says it is a summary, and the project field says which
                // project. So the headline stands alone, and the old name is
                // kept only for the case it was written for — a summary with
                // nothing worth lifting, which still needs to be called
                // something.
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
        // The agent that wrote it is the only one who can name it, and only
        // while it still remembers what the session was for.
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
        // An explicit project is scoped and validated; a merely detected one is
        // reported as context but never turns a diagnostic into an error,
        // because a doctor that refuses to run where you are is useless.
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
