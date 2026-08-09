use super::*;

/// How thoroughly something went, for saying so afterwards.
pub(super) fn gone(language: crate::settings::Interface, hard: bool) -> &'static str {
    let say = crate::i18n::screens(language);
    if hard { say.gone_permanently } else { say.gone }
}

/// What a delete would destroy, counted from the store.
///
/// Counted rather than estimated. A session row carries the number of
/// observations the list was narrowed to — under a search, the number of
/// matches — and a project's row on the dashboard is a name and nothing else.
/// A confirmation is worth having only if the number on it is the number that
/// is about to go.
pub(super) fn confirmation(
    store: &Store,
    language: crate::settings::Interface,
    what: &Target,
    hard: bool,
) -> Result<PendingAction, StoreError> {
    let say = crate::i18n::screens(language);
    // Prompts have no tombstone: the store deletes them outright whichever key
    // was pressed, and that is true of the prompts inside a session or a project
    // as well. So a soft delete is only half soft whenever prompts are involved,
    // and the window has to say which half.
    let mut prompts_go_for_good = false;
    let (heading, mut detail) = match what {
        Target::Observation(id) => {
            let observation = store.get_observation(*id)?;
            (
                fill(say.delete_memory, "id", id),
                vec![truncate(&observation.title, 60)],
            )
        }
        Target::Prompt(id) => {
            let prompt = store.get_prompt(*id)?;
            prompts_go_for_good = true;
            (
                fill(say.delete_prompt, "id", id),
                vec![truncate(
                    prompt.content.trim().replace('\n', " ").trim(),
                    60,
                )],
            )
        }
        Target::Session(id) => {
            let (observations, prompts) = store.session_counts(id)?;
            prompts_go_for_good = prompts > 0;
            (
                fill(say.delete_session, "id", truncate(id, 40)),
                vec![
                    fill(say.count_memories, "count", observations),
                    fill(say.count_prompts, "count", prompts),
                ],
            )
        }
        Target::Project(name) => {
            let stats = store
                .list_projects_with_stats()?
                .into_iter()
                .find(|project| project.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| StoreError::ProjectNotFound(name.clone()))?;
            prompts_go_for_good = stats.prompt_count > 0;
            (
                fill(say.delete_project, "name", truncate(name, 40)),
                vec![
                    fill(say.count_memories, "count", stats.observation_count),
                    fill(say.count_sessions, "count", stats.session_count),
                    fill(say.count_prompts, "count", stats.prompt_count),
                ],
            )
        }
    };
    // Said once, at the end, rather than folded into the heading: it is the part
    // that decides whether somebody presses y, and it should be the last thing
    // read before they do.
    detail.push(String::new());
    detail.push(
        match (hard, prompts_go_for_good) {
            (true, _) => say.delete_permanent_warning,
            (false, true) => say.delete_prompts_warning,
            (false, false) => say.delete_recoverable,
        }
        .to_owned(),
    );
    Ok(PendingAction {
        heading,
        detail,
        action: Action::Delete {
            what: what.clone(),
            hard,
        },
    })
}

pub(super) fn apply_action(app: &mut App, store: &mut Store, action: Action) -> bool {
    let say = crate::i18n::screens(app.interface);
    let language = app.interface;
    match action {
        Action::None => false,
        Action::Quit => true,
        Action::Copy(text) => {
            match copy_to_clipboard(&text) {
                Ok(()) => {
                    app.status = Some(StatusMessage {
                        text: fill(say.copied_to_clipboard, "count", text.chars().count()),
                        is_error: false,
                    });
                }
                Err(error) => app.set_error(error),
            }
            false
        }
        // Counted from what is on screen rather than from the store, because
        // these are the store's own totals and they are already loaded. The
        // agents are counted from the same probe the setup page uses.
        Action::ConfirmUninstall => {
            let agents = crate::setup::supported_agents()
                .iter()
                .filter(|agent| crate::setup::is_configured(agent.slug, &app.setup_probe))
                .count();
            app.status = None;
            app.pending = Some(PendingAction {
                heading: say.uninstall_heading.to_owned(),
                detail: vec![
                    fill(say.count_memories, "count", app.stats.total_observations),
                    fill(say.uninstall_agents, "count", agents),
                    app.database_path.clone(),
                    String::new(),
                    say.uninstall_warning.to_owned(),
                ],
                action: Action::Uninstall,
            });
            false
        }
        // Agreed to. The work happens outside this process, for the reason on
        // `Action::Uninstall`: the database is open here.
        Action::Uninstall => {
            app.exit = crate::tui::Exit::Uninstall;
            true
        }
        Action::Confirm { what, hard } => {
            match confirmation(store, language, &what, hard) {
                Ok(pending) => {
                    app.status = None;
                    app.pending = Some(pending);
                }
                Err(error) => app.set_error(error),
            }
            false
        }
        Action::Delete { what, hard } => {
            let done = match &what {
                Target::Observation(id) => store.delete_observation(*id, hard).map(|()| {
                    fill(
                        &fill(say.deleted_memory, "id", id),
                        "gone",
                        gone(language, hard),
                    )
                }),
                Target::Prompt(id) => store
                    .delete_prompt(*id)
                    .map(|()| fill(say.deleted_prompt, "id", id)),
                Target::Session(id) => store.delete_session_and_contents(id, hard).map(|result| {
                    let text = fill(say.deleted_session, "id", id);
                    let text = fill(&text, "gone", gone(language, hard));
                    let text = fill(&text, "memories", result.observations_deleted);
                    fill(&text, "prompts", result.prompts_deleted)
                }),
                Target::Project(name) => store.delete_project(name, hard).map(|result| {
                    let text = fill(say.deleted_project, "name", name);
                    let text = fill(&text, "gone", gone(language, hard));
                    let text = fill(&text, "memories", result.observations_deleted);
                    let text = fill(&text, "sessions", result.sessions_deleted);
                    let mut text = fill(&text, "prompts", result.prompts_deleted);
                    // Said out loud rather than left to be noticed: a delete
                    // that quietly left something behind is worse than one that
                    // reports it.
                    if result.sessions_kept > 0 {
                        text.push_str(&fill(say.sessions_kept, "count", result.sessions_kept));
                    }
                    text
                }),
            };
            match done {
                Ok(text) => {
                    app.forget(&what);
                    match app.refresh(store) {
                        Ok(()) => {
                            app.status = Some(StatusMessage {
                                text,
                                is_error: false,
                            });
                        }
                        Err(error) => app.set_error(error),
                    }
                }
                Err(error) => app.set_error(error),
            }
            false
        }
        Action::OpenObservation(id) => {
            match store.get_observation(id) {
                Ok(observation) => {
                    // Failing to reach the graph costs the annotation, not the
                    // memory somebody asked to read.
                    let caveats = store
                        .caveats_for(std::slice::from_ref(&observation.sync_id))
                        .unwrap_or_default()
                        .remove(&observation.sync_id)
                        .unwrap_or_default();
                    app.open_detail(observation, caveats);
                }
                Err(error) => app.set_error(error),
            }
            false
        }
        Action::OpenSession(id) => {
            // The summary is already in hand from the list; only what the
            // session recorded has to be fetched.
            let Some(summary) = app
                .sessions
                .iter()
                .find(|session| session.id == id)
                .cloned()
            else {
                return false;
            };
            app.session_entry_offset = 0;
            match store.paged_session_observations(&id, 0, WINDOW) {
                Ok(entries) => app.open_session(summary, entries),
                Err(error) => app.set_error(error),
            }
            false
        }
        Action::LoadTimeline(id) => {
            match store.timeline(id, Some(20), Some(20)) {
                Ok(timeline) => app.open_timeline(timeline),
                Err(error) => app.set_error(error),
            }
            false
        }
        Action::Narrow => {
            match app.narrow(store) {
                // No status line. It would be rewritten on every letter, and
                // the counters above already say how many of each matched.
                Ok(()) => app.status = None,
                Err(error) => app.set_error(error),
            }
            false
        }
        Action::Refresh => {
            match app.refresh(store) {
                Ok(()) => {
                    app.status = Some(StatusMessage {
                        text: match app.query.trim() {
                            "" => say.data_refreshed.to_owned(),
                            query => {
                                let text = fill(say.refreshed_query, "query", query);
                                let text = fill(&text, "observations", app.recent_total);
                                let text = fill(&text, "sessions", app.session_total);
                                fill(&text, "prompts", app.prompt_total)
                            }
                        },
                        is_error: false,
                    });
                }
                Err(error) => {
                    // Back into the input if the query is what the store
                    // objected to, so the complaint lands where the fix is.
                    if !app.query.trim().is_empty() {
                        app.focus = Focus::Query;
                    }
                    app.set_error(error);
                }
            }
            false
        }
    }
}

/// Copies text using the OSC 52 terminal escape sequence.
///
/// This works in Windows Terminal, iTerm2, kitty, WezTerm, tmux, and over SSH,
/// and needs no platform clipboard library.
fn copy_to_clipboard(text: &str) -> io::Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::io::Write;

    let encoded = STANDARD.encode(text.as_bytes());
    let mut stdout = io::stdout();
    write!(stdout, "\x1b]52;c;{encoded}\x07")?;
    stdout.flush()
}
