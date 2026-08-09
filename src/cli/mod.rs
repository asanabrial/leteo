use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use crate::{
    AddObservation, AddPrompt, SearchMode, SearchOptions, Store, StoreConfig,
    memory::model::{ListDeferredOptions, ListRelationsOptions, ProjectStats, ScanOptions},
};

mod args;
mod bootstrap;
mod projects;

pub use args::Cli;
// Exposed for the guard in `setup::tests` that holds the hooks the installer
// writes to the subcommands this binary parses.
#[cfg(test)]
pub(crate) use args::HookEventArgument;
use args::*;
use bootstrap::*;
use projects::*;

impl From<GraphConfigArgument> for crate::obsidian::GraphConfig {
    fn from(value: GraphConfigArgument) -> Self {
        match value {
            GraphConfigArgument::Preserve => Self::Preserve,
            GraphConfigArgument::Force => Self::Force,
            GraphConfigArgument::Skip => Self::Skip,
        }
    }
}

impl From<HookEventArgument> for crate::hooks::HookEvent {
    fn from(value: HookEventArgument) -> Self {
        match value {
            HookEventArgument::SessionStart => Self::SessionStart,
            HookEventArgument::PostCompaction => Self::PostCompaction,
            HookEventArgument::UserPromptSubmit => Self::UserPromptSubmit,
            HookEventArgument::SubagentStop => Self::SubagentStop,
            HookEventArgument::SessionStop => Self::SessionStop,
        }
    }
}

impl std::fmt::Display for MatchMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::All => formatter.write_str("all"),
            Self::Any => formatter.write_str("any"),
        }
    }
}

pub async fn run(cli: Cli) -> Result<()> {
    match &cli.command {
        Command::CurrentProject => {
            return print_json(&crate::project::detect_current_project());
        }
        // Adoption replaces the database file, so it has to run before anything
        // opens it.
        Command::Import {
            from_engram: true,
            source,
            dry_run,
            ..
        } => {
            let source = match source {
                Some(path) => absolutize(path)?,
                None => crate::engram::default_database().context(
                    "no Engram database found at ~/.engram/engram.db; pass --source to name one",
                )?,
            };
            let target = store_config(&cli)?.database_path;
            return print_json(&crate::engram::adopt(&source, &target, *dry_run)?);
        }
        Command::Setup {
            agent,
            interactive,
            dry_run,
            instructions,
            hooks,
            tools,
            language,
            context,
            uninstall,
        } => {
            // Written before anything else, and whatever else this run does.
            // `--language` alone is a complete command: somebody changing which
            // language their memories are written in should not have to
            // reconfigure an agent to say so.
            if let Some(language) = language {
                let data_dir = data_directory(&cli)?;
                let settings = crate::settings::Settings {
                    language: Some(language.trim().to_owned()).filter(|l| !l.is_empty()),
                    ..crate::settings::load(&data_dir)
                };
                crate::settings::save(&data_dir, &settings)?;
            }
            // The same, for how much a session opens with. Refused rather than
            // ignored when it is not one of the three: a typo that silently
            // kept the old size would be found weeks later, by wondering why
            // the block never got shorter.
            if let Some(context) = context {
                let size = crate::settings::ContextSize::parse(context).with_context(|| {
                    format!("unknown context size {context:?}: expected slim, full or deep")
                })?;
                let data_dir = data_directory(&cli)?;
                let settings = crate::settings::Settings {
                    context_size: Some(size),
                    ..crate::settings::load(&data_dir)
                };
                crate::settings::save(&data_dir, &settings)?;
            }
            if let Some(agent) = agent.as_deref() {
                let options = crate::setup::SetupOptions {
                    dry_run: *dry_run,
                    install_instructions: *instructions,
                    install_hooks: *hooks,
                    tools: tools.clone(),
                    ..crate::setup::SetupOptions::default()
                };
                if *uninstall {
                    print_json(&crate::setup::uninstall(agent, &options)?)?;
                } else {
                    print_json(&crate::setup::setup(agent, &options)?)?;
                }
            // Both ends have to be a terminal. Reading keys needs a real
            // stdin, so going interactive on output alone leaves the flow
            // waiting for keys that can never arrive.
            } else if *interactive
                || (std::io::IsTerminal::is_terminal(&std::io::stdout())
                    && std::io::IsTerminal::is_terminal(&std::io::stdin()))
            {
                // Someone is watching, so walk them through it. A script gets
                // the JSON below instead, because turning that into a prompt
                // would hang it.
                let database = store_config(&cli)?.database_path;
                let held = stored_observations(&cli);
                // Adoption is only a question when there is nothing to lose.
                // When there is, say so before the wizard starts rather than
                // leaving Engram unmentioned: somebody part-way through a
                // migration would otherwise conclude Leteo cannot see it.
                if let Some(held) = held.filter(|held| *held > 0)
                    && let Some(found) = crate::engram::default_database()
                        .and_then(|path| crate::engram::inspect(&path).ok())
                        .filter(|installation| !installation.is_empty())
                {
                    println!("{}\n", crate::setup::wizard::adoption_note(&found, held));
                }
                // `Some(0)`, not "zero or unknown". A store nobody could read
                // is not an empty one, and reading it as empty got both of the
                // decisions behind this flag wrong at once: it offered to
                // migrate Engram over the top of it, and it withheld the
                // warning that changing the language leaves what is already
                // saved in the language it was written in.
                let offer = crate::setup::wizard::offer(
                    &database,
                    store_is_known_empty(&cli),
                    &crate::setup::SetupOptions::default(),
                );
                crate::setup::wizard::run_interactive(offer)?;
            } else {
                let agents = crate::setup::supported_agents()
                    .iter()
                    .map(|agent| {
                        serde_json::json!({
                            "slug": agent.slug,
                            "display_name": agent.display_name,
                        })
                    })
                    .collect::<Vec<_>>();
                // Someone arriving from Engram has memories worth keeping and
                // no reason to know that Leteo can take them. Say so here,
                // where they are already looking, rather than in a document.
                print_json(&serde_json::json!({
                    "agents": agents,
                    "engram": engram_offer(&cli),
                }))?;
            }
            return Ok(());
        }
        Command::Uninstall { yes } => {
            // Without `--yes` this is a dry run rather than a prompt. The
            // callers that matter — `uninstall.ps1`, `uninstall.sh`, and
            // Windows running the `UninstallString` from Settings — may have no
            // console at all, and a question nobody can answer is worse than no
            // question. Those ask first, in a place a person is looking.
            let options = crate::setup::SetupOptions {
                dry_run: !*yes,
                ..crate::setup::SetupOptions::default()
            };
            let data_dir = data_directory(&cli)?;
            let removed = crate::setup::uninstall_everything(&options, &data_dir);
            print_json(&removed)?;
            // A partial uninstall must not read as a clean one. Somebody who
            // ran this to get Leteo off a machine needs to know an agent still
            // has it, and a script needs a non-zero status to notice.
            if *yes && !removed.complete() {
                anyhow::bail!("Leteo was not removed completely; see the report above");
            }
            return Ok(());
        }
        Command::Cloud {
            command: CloudCommand::Serve,
        } => {
            crate::cloud::CloudServer::from_config(crate::cloud::CloudConfig::from_env())
                .await?
                .serve()
                .await?;
            return Ok(());
        }
        Command::Cloud {
            command: CloudCommand::Health { server, token },
        } => {
            let persisted = crate::cloud::ClientConfig::load(crate::cloud::ClientConfig::path_in(
                data_directory(&cli)?,
            ))?;
            let server = server
                .clone()
                .filter(|server| !server.trim().is_empty())
                .unwrap_or_else(|| persisted.server.clone());
            if server.trim().is_empty() {
                anyhow::bail!(
                    "no cloud server configured; pass --server URL or run: leteo cloud config set --server URL"
                );
            }
            let token = token
                .clone()
                .filter(|token| !token.trim().is_empty())
                .unwrap_or_else(|| persisted.token.clone());
            let remote = crate::cloud::RemoteClient::new(&server, &token)?;
            print_json(&remote.health().await?)?;
            return Ok(());
        }
        Command::Cloud {
            command: CloudCommand::Admin { command },
        } => {
            run_cloud_admin(command).await?;
            return Ok(());
        }
        Command::Cloud {
            command: CloudCommand::Config { command },
        } => {
            let path = crate::cloud::ClientConfig::path_in(data_directory(&cli)?);
            match command {
                CloudConfigCommand::Show => {
                    let config = crate::cloud::ClientConfig::load(&path)?;
                    print_json(&serde_json::json!({
                        "path": path.to_string_lossy(),
                        "exists": path.exists(),
                        "config": config.redacted(),
                    }))?;
                }
                CloudConfigCommand::Set {
                    server,
                    token,
                    project,
                    poll_interval,
                    enable,
                    disable,
                } => {
                    let mut config = crate::cloud::ClientConfig::load(&path)?;
                    if let Some(server) = server {
                        config.server = server.trim().to_owned();
                    }
                    if let Some(token) = token {
                        config.token = token.trim().to_owned();
                    }
                    if !project.is_empty() {
                        config.projects = project.clone();
                        // And the enrolment table with it.
                        //
                        // Two lists decide whether a memory reaches the cloud,
                        // and they are read at opposite ends: this one says
                        // what the loop may push, and the table in the store
                        // says what is written down to be pushed at all.
                        // Setting one alone left `cloud status` reporting an
                        // enabled, runnable replication of a project whose
                        // every memory was journalled nowhere.
                        //
                        // `cloud enroll` has kept the two aligned from the
                        // other direction since it was written; this is the
                        // same rule from this side. Enrolling queues what the
                        // project already holds, so naming it here replicates
                        // its history rather than only what comes next — and
                        // dropping one from the list stops its journal, which
                        // is what dropping it means.
                        //
                        // The store is opened here rather than for the whole
                        // command: naming a project is the only part of this
                        // that has anything to record, and a `--server` or a
                        // `--token` on their own have no business creating a
                        // database.
                        let mut store = Store::open(store_config(&cli)?)
                            .context("open Leteo store to enrol the configured projects")?;
                        reconcile_enrolment(&mut store, &config.projects)?;
                    }
                    if let Some(poll_interval) = poll_interval {
                        config.poll_interval_seconds = Some(*poll_interval);
                    }
                    if *enable {
                        config.enabled = true;
                    }
                    if *disable {
                        config.enabled = false;
                    }
                    config.save(&path)?;
                    let config = crate::cloud::ClientConfig::load(&path)?;
                    print_json(&serde_json::json!({
                        "path": path.to_string_lossy(),
                        "config": config.redacted(),
                    }))?;
                }
                CloudConfigCommand::Clear => {
                    let removed = match std::fs::remove_file(&path) {
                        Ok(()) => true,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                        Err(error) => {
                            return Err(error)
                                .with_context(|| format!("remove {}", path.display()));
                        }
                    };
                    print_json(&serde_json::json!({
                        "path": path.to_string_lossy(),
                        "removed": removed,
                    }))?;
                }
            }
            return Ok(());
        }
        _ => {}
    }
    let mut config = store_config(&cli)?;
    // A hook gives up before the agent that launched it does. The number is on
    // the event — see `HookEvent::store_wait` — because it is the event that
    // knows how long anybody is waiting.
    if let Command::Hook { event, .. } = &cli.command {
        config.busy_timeout = crate::hooks::HookEvent::from(*event).store_wait();
    }
    let data_directory = config
        .database_path
        .parent()
        .map(Path::to_path_buf)
        .context("the Leteo database path has no parent directory")?;
    let cloud_config_path = crate::cloud::ClientConfig::path_in(&data_directory);
    // A hook answers even when the store cannot be opened at all.
    //
    // The hook module says it plainly — "hooks sit on the agent's critical
    // path: a malformed payload or a store problem must never block the user's
    // prompt" — and then the store was opened out here, before the hook was
    // reached, so the promise was made in one file and broken in another. A
    // database corrupted, on a disk that filled, on a drive that went away:
    // `leteo hook user-prompt-submit` printed a Rust error to stderr and exited
    // 1, on every prompt, for as long as the file stayed broken.
    //
    // So the answer is the empty response and a zero exit, which is what a hook
    // with nothing to add already returns, and the reason goes to stderr where
    // the agent's verbose mode shows it. `leteo doctor` is the place to look
    // next, and it is the one command that has to keep failing loudly.
    let store = Store::open(config.clone());
    if let (Command::Hook { verbose, .. }, Err(error)) = (&cli.command, &store) {
        eprintln!("leteo hook: the store could not be opened: {error:#}");
        if *verbose {
            print_json(&serde_json::json!({
                "warnings": [format!("open Leteo store: {error:#}")],
            }))?;
        } else {
            println!("{{}}");
        }
        return Ok(());
    }
    let mut store = store.context("open Leteo store")?;
    match cli.command {
        Command::SessionStart {
            id,
            project,
            directory,
        } => {
            print_json(&store.create_session(&id, &project, &directory.to_string_lossy())?)?;
        }
        Command::SessionEnd { id, summary } => {
            print_json(&store.end_session(&id, summary.as_deref())?)?;
        }
        Command::Save {
            title,
            content,
            r#type,
            session,
            project,
            scope,
            topic_key,
            tool_name,
        } => {
            let session = resolve_write_session(&mut store, session, project)?;
            // The same attribution the tool makes, on the other door into the
            // same table. This wrote `None` unconditionally, with no comment
            // saying why, beside sixty lines in `mem_save` arguing which
            // question a save may be hung on — and a memory typed at a terminal
            // in the middle of a conversation answers that conversation.
            let prompt_sync_id =
                store.prompt_behind_a_save(&session.id, &session.project, session.named);
            let project = Some(session.project);
            let saved = store.add_observation(AddObservation {
                session_id: session.id,
                kind: r#type,
                title,
                content,
                tool_name,
                project,
                scope,
                topic_key,
                prompt_sync_id,
            })?;
            // The same sentence the tool answers with, on the channel a person
            // reads. A type outside the eight is kept — see `UNFILED_KIND_HINT`
            // for why guessing at a fold is worse — and the memory becomes one
            // a search narrowed by type can never return.
            if !crate::memory::rules::is_searchable_kind(&saved.observation.kind) {
                eprintln!("leteo save: {}", crate::mcp::UNFILED_KIND_HINT);
            }
            print_json(&saved)?;
        }
        Command::Search {
            query,
            project,
            all_projects,
            r#type,
            scope,
            limit,
            match_mode,
        } => {
            let scoped = read_scope(project, all_projects);
            let options = SearchOptions {
                kind: r#type,
                project: scoped.project.clone(),
                scope,
                limit: Some(limit),
                mode: match match_mode {
                    MatchMode::All => SearchMode::All,
                    MatchMode::Any => SearchMode::Any,
                },
            };
            let cap = store.max_search_results();
            let (found, more) = store.search_with_more(&query, options.clone())?;
            // The same sentence the MCP tool answers with, on the channel a
            // person reads rather than the one a script parses.
            //
            // An empty result reads like "this was never saved" and is usually
            // "your words did not match": memories are written by an agent and
            // are usually in English while the question often is not, and on a
            // real store an English term finds up to twenty memories where its
            // Spanish equivalent finds none. An agent has been told that since
            // the hint was written; somebody at a terminal got `[]`.
            //
            // On stderr because stdout is a JSON array that something may be
            // parsing, and because a search answered by relaxing the question
            // is worth saying out loud either way.
            //
            // And when the directory chose the project, the empty answer has a
            // second possible reason, which the hint above would name wrongly:
            // the words matched, in a project this is not. Saying "try fewer,
            // more distinctive words" to somebody standing in the wrong
            // directory sends them to rewrite a question that was already
            // right. So the store is asked once more, unnarrowed, and only
            // where that finds something does the reason change — a search
            // that would come back empty either way keeps the original hint.
            //
            // The extra query is paid only on an empty answer, and only when
            // nobody named a project.
            if found.is_empty() {
                let elsewhere = scoped
                    .inferred
                    .then(|| {
                        store.search(
                            &query,
                            SearchOptions {
                                project: None,
                                ..options
                            },
                        )
                    })
                    .transpose()?
                    .unwrap_or_default();
                match (scoped.project.as_deref(), elsewhere.is_empty()) {
                    (Some(project), false) => eprintln!(
                        "leteo search: {}",
                        // Contra el mismo tope con el que se contó: la lista
                        // vuelve topada al límite de quien preguntó, así que el
                        // número no puede pasar de ahí y lo dice.
                        crate::mcp::no_match_here_hint(
                            project,
                            elsewhere.len(),
                            options.limit.unwrap_or(crate::store::DEFAULT_SEARCH_LIMIT),
                            "--all-projects",
                        )
                    ),
                    _ => eprintln!("leteo search: {}", crate::mcp::NO_MATCH_HINT),
                }
            } else if found.iter().any(|result| result.partial) {
                eprintln!("leteo search: {}", crate::mcp::PARTIAL_MATCH_HINT);
            } else if more && found.len() >= cap {
                // `--limit 50` returns twenty and used to say nothing, so the
                // answer read as twenty matches. The branch below cannot cover
                // it: its advice — ask again with a higher limit — is the one
                // thing that cannot help once the store's own maximum is what
                // ended the list. Asked of what came back rather than of what
                // was requested, for the reason `mem_search` gives.
                eprintln!("leteo search: {}", crate::mcp::clamped_hint(cap));
            } else if more {
                eprintln!("leteo search: {}", crate::mcp::MORE_MATCHED_HINT);
            }
            print_json(&found)?;
        }
        Command::Prompt {
            content,
            session,
            project,
        } => {
            let session = resolve_write_session(&mut store, session, project)?;
            print_json(&store.add_prompt(AddPrompt {
                session_id: session.id,
                content,
                project: Some(session.project),
            })?)?;
        }
        Command::Recent {
            project,
            all_projects,
            limit,
            summaries,
        } => {
            let scope = read_scope(project, all_projects);
            let found =
                store.recent_observations(scope.project.as_deref(), Some(limit), summaries)?;
            if found.is_empty() {
                say_where_it_is_empty(&store, &scope);
            } else if !summaries {
                // Said only when some were actually held back, and only then:
                // a project with no summaries has nothing to explain.
                // How many fall inside the same window, not how much longer the
                // list got: both are cut to `limit`, so subtracting one from
                // the other is always zero and says nothing.
                let with = store
                    .recent_observations(scope.project.as_deref(), Some(limit), true)
                    .map(|all| {
                        all.iter()
                            .filter(|held| held.kind == crate::memory::model::SESSION_SUMMARY)
                            .count()
                    })
                    .unwrap_or_default();
                if with > 0 {
                    eprintln!(
                        "leteo recent: {with} session summaries left out — pass --summaries to see them."
                    );
                }
            }
            print_json(&found)?;
        }
        Command::Delete { command } => match command {
            DeleteCommand::Observation { id, hard } => {
                store.delete_observation(id, hard)?;
                print_json(&serde_json::json!({
                    "id": id,
                    "deleted": true,
                    "hard_delete": hard,
                }))?;
            }
            DeleteCommand::Session { id } => {
                store.delete_session(&id)?;
                print_json(&serde_json::json!({ "id": id, "deleted": true }))?;
            }
            DeleteCommand::Prompt { id } => {
                store.delete_prompt(id)?;
                print_json(&serde_json::json!({ "id": id, "deleted": true }))?;
            }
            DeleteCommand::Project { name, hard } => {
                print_json(&store.delete_project(&name, hard)?)?;
            }
        },
        Command::Projects { command } => run_projects(&mut store, command)?,
        Command::Timeline { id, before, after } => {
            print_json(&store.timeline(id, before, after)?)?;
        }
        Command::Context {
            project,
            project_flag,
            all_projects,
            scope,
            limit,
        } => {
            let read = read_scope(project_flag.or(project), all_projects);
            let context = crate::recall::assemble(
                &store,
                read.project.as_deref(),
                scope.as_deref(),
                limit.unwrap_or_else(|| crate::recall::default_memories(&store)),
            )?;
            if context.trim().is_empty() {
                say_where_it_is_empty(&store, &read);
            }
            print_json(&serde_json::json!({ "context": context }))?;
        }
        Command::Doctor {
            check,
            project,
            repair,
        } => {
            // Before the report, so what comes back describes the store as it
            // is now rather than as it was when somebody asked for help.
            //
            // The triggers go back first and the rebuild follows, in that
            // order: the rebuild is what recovers the edits that happened
            // while a trigger was missing, and doing it first would leave the
            // index correct for a moment and stale again by the next write.
            let restored = repair
                .then(|| store.restore_full_text_triggers())
                .transpose()?;
            let rebuilt = repair
                .then(|| store.rebuild_full_text_indexes())
                .transpose()?;
            let rehashed = repair.then(|| store.recompute_stale_hashes()).transpose()?;
            let (report, stats) = store.doctor_scoped(check.as_deref(), project.as_deref())?;
            // The report stays at the top level so existing readers keep
            // working; the scoping fields are additions beside it.
            let mut output = serde_json::to_value(&report)?;
            if let Some(object) = output.as_object_mut() {
                if let Some(restored) = restored {
                    object.insert(
                        "restored_triggers".to_owned(),
                        serde_json::to_value(&restored)?,
                    );
                }
                if let Some(rebuilt) = rebuilt {
                    object.insert("rebuilt".to_owned(), serde_json::to_value(&rebuilt)?);
                }
                if let Some(rehashed) = rehashed {
                    object.insert("rehashed".to_owned(), serde_json::to_value(rehashed)?);
                }
                if let Some(check) = check {
                    object.insert("check".to_owned(), serde_json::Value::String(check));
                }
                if let Some(stats) = stats {
                    object.insert(
                        "project".to_owned(),
                        serde_json::Value::String(stats.name.clone()),
                    );
                    object.insert("project_stats".to_owned(), serde_json::to_value(&stats)?);
                }
                // Whether the hooks are installed, and whether they will fire.
                //
                // The README has always sent people here when a hook goes
                // quiet, and until now doctor could only answer for SQLite —
                // it would report a perfectly healthy store to somebody whose
                // hooks had never run once. `healthy` follows, because a store
                // nothing is writing to is not a healthy installation however
                // clean its indexes are.
                //
                // Only at the CLI. `mem_doctor` asks the same store the same
                // questions, but an agent inspecting the hooks that invoke it
                // cannot act on the answer, and the person who can is the one
                // standing at a terminal.
                let hooks = crate::setup::hook_health(&crate::setup::SetupOptions::default());
                let issues: Vec<&str> = hooks
                    .iter()
                    .filter_map(|agent| agent.issue.as_deref())
                    .collect();
                if !issues.is_empty() {
                    object.insert("healthy".to_owned(), serde_json::Value::Bool(false));
                    if let Some(serde_json::Value::Array(existing)) = object.get_mut("issues") {
                        existing.extend(
                            issues
                                .iter()
                                .map(|issue| serde_json::Value::String((*issue).to_owned())),
                        );
                    }
                }
                object.insert("agent_hooks".to_owned(), serde_json::to_value(&hooks)?);
            }
            print_json(&output)?;
        }
        Command::Conflicts { command } => run_conflicts(&mut store, command).await?,
        Command::Export { project, output } => {
            let data = store.export_scoped(project.as_deref())?;
            let json = serde_json::to_string_pretty(&data)?;
            match output {
                Some(path) => {
                    let path = absolutize(&path)?;
                    std::fs::write(&path, json.as_bytes())
                        .with_context(|| format!("write export to {}", path.display()))?;
                    print_json(&serde_json::json!({
                        "output": path.to_string_lossy(),
                        "sessions": data.sessions.len(),
                        "observations": data.observations.len(),
                        "prompts": data.prompts.len(),
                        "relations": data.relations.len(),
                    }))?;
                }
                None => println!("{json}"),
            }
        }
        Command::Import { file, input, .. } => {
            let file = input
                .or(file)
                .context("import needs a file, or --from-engram")?;
            let path = absolutize(&file)?;
            let json = std::fs::read_to_string(&path)
                .with_context(|| format!("read import file {}", path.display()))?;
            print_json(&store.import_json(&json)?)?;
        }
        Command::ObsidianExport {
            vault,
            project,
            limit,
            graph_config,
        } => {
            print_json(&crate::obsidian::export(
                &store,
                &crate::obsidian::ExportOptions {
                    vault: absolutize(&vault)?,
                    project,
                    limit,
                    graph_config: graph_config.into(),
                },
            )?)?;
        }
        Command::Stats => print_json(&store.stats()?)?,
        Command::Serve => {
            let autosync = start_background_autosync(&config, &cloud_config_path)?;
            if !autosync.is_running() {
                anyhow::bail!(
                    "cloud replication is not configured; run `leteo cloud config set` first"
                );
            }
            // Nothing else to do on this thread: replication runs on its own,
            // and Ctrl-C is how it ends.
            tokio::signal::ctrl_c().await?;
            autosync.shutdown().await;
        }
        Command::Mcp { tools, project } => {
            let autosync = start_background_autosync(&config, &cloud_config_path)?;
            let served = crate::mcp::run_stdio_with_options(
                Arc::new(Mutex::new(store)),
                crate::mcp::McpOptions {
                    default_project: project,
                    tools,
                },
            )
            .await;
            autosync.shutdown().await;
            served?;
        }
        Command::Tui => {
            // The dashboard cannot carry out an uninstall itself: this process
            // holds `leteo.db` open, and Windows will not delete an open file.
            // So it collects the agreement, closes, and the removal happens
            // here — after the store is dropped and the terminal is back.
            if crate::tui::run(&mut store)? == crate::tui::Exit::Uninstall {
                drop(store);
                let removed = crate::setup::uninstall_everything(
                    &crate::setup::SetupOptions::default(),
                    &data_directory,
                );
                print_json(&removed)?;
                if !removed.complete() {
                    anyhow::bail!("Leteo was not removed completely; see the report above");
                }
                return Ok(());
            }
        }
        Command::Hook { event, verbose } => {
            // Hooks sit on the agent's critical path: a malformed payload or a
            // store problem must never block the user's prompt.
            //
            // But carrying on is not the same as saying nothing. Falling back
            // to an empty payload silently is what hid a real bug for two
            // rounds of this review: a `serde` alias turned Codex's ordinary
            // payload into a duplicate field, every hook parsed as an empty
            // `HookInput`, and each one reported success having done nothing.
            // The store filled with sessions nobody could find and prompts that
            // were never saved, and nothing anywhere said why.
            //
            // So the fallback stays and the reason travels with it. The hook
            // still answers, still never blocks; the outcome now carries a
            // warning naming what could not be read, which `--verbose` prints
            // and `leteo doctor` is the place to look next.
            let (input, unreadable) = match crate::hooks::read_input(std::io::stdin()) {
                Ok(input) => (input, None),
                Err(error) => (
                    crate::hooks::HookInput::default(),
                    Some(format!("hook payload could not be read: {error:#}")),
                ),
            };
            match crate::hooks::run(&mut store, event.into(), &input) {
                Ok(mut outcome) => {
                    if let Some(warning) = unreadable {
                        outcome.warnings.push(warning);
                    }
                    // Nine places in the hooks collect a warning when something
                    // recoverable goes wrong — a prompt that could not be
                    // saved, a session that could not be created, a sync import
                    // that failed. Every one of them went into `warnings`, and
                    // `HookOutcome::response` does not carry that field. So in
                    // the mode hooks actually run in they went nowhere: the
                    // agent got `{}` and the person got nothing.
                    //
                    // Standard error is the right channel rather than the
                    // response. It stays out of the agent's context, which is
                    // what `response` is for and what a warning has no business
                    // spending, and it is where an agent's own hook logs look.

                    for warning in &outcome.warnings {
                        eprintln!("leteo hook {}: {warning}", outcome.event);
                    }
                    if verbose {
                        print_json(&outcome)?;
                    } else {
                        print_json(&outcome.response())?;
                    }
                }
                Err(error) => {
                    eprintln!("leteo hook: {error}");
                    print_json(&serde_json::json!({}))?;
                }
            }
        }
        Command::Cloud {
            command:
                CloudCommand::Sync {
                    server,
                    token,
                    project,
                },
        } => {
            let persisted = crate::cloud::ClientConfig::load(&cloud_config_path)?;
            let server = server
                .filter(|server| !server.trim().is_empty())
                .unwrap_or_else(|| persisted.server.clone());
            let token = token
                .filter(|token| !token.trim().is_empty())
                .unwrap_or_else(|| persisted.token.clone());
            let projects = match project {
                Some(project) => vec![crate::memory::normalize::project(&project)],
                None => persisted.projects.clone(),
            };
            if server.trim().is_empty() || token.trim().is_empty() || projects.is_empty() {
                persisted.require_runnable()?;
            }
            let remote = crate::cloud::RemoteClient::new(&server, &token)?;
            let config = crate::cloud::AutosyncConfig {
                allowed_projects: projects,
                ..crate::cloud::AutosyncConfig::default()
            };
            let mut autosync = crate::cloud::Autosync::new(&mut store, remote, config)?;
            autosync.run_cycle().await?;
            print_json(autosync.status())?;
        }
        Command::Cloud {
            command: CloudCommand::Status,
        } => {
            let config = crate::cloud::ClientConfig::load(&cloud_config_path)?;
            let state = store.get_sync_state(crate::cloud::CLOUD_SYNC_TARGET)?;
            let (deferred, dead) = store.deferred_sync_counts()?;
            print_json(&serde_json::json!({
                "config": config.redacted(),
                "enrolled_projects": store.enrolled_projects()?,
                "pending_mutations": store
                    .pending_sync_mutation_count(crate::cloud::CLOUD_SYNC_TARGET)?,
                // The question the count does not answer: a hundred waiting
                // since this morning is a busy peer, a hundred waiting since
                // March is replication that stopped. Null when nothing waits.
                "pending_since": store
                    .oldest_pending_mutation(crate::cloud::CLOUD_SYNC_TARGET)?,
                "target": state,
                "deferred_count": deferred,
                "dead_count": dead,
            }))?;
        }
        Command::Cloud {
            command: CloudCommand::Enroll { project, remove },
        } => {
            let project = resolve_project(project)?;
            let changed = if remove {
                store.unenroll_project(&project)?
            } else {
                store.enroll_project(&project)?
            };
            // Keep the replicated project list and the enrollment table aligned
            // so the background loop pushes exactly what was enrolled.
            let mut config = crate::cloud::ClientConfig::load(&cloud_config_path)?;
            config.projects = store.enrolled_projects()?;
            config.save(&cloud_config_path)?;
            print_json(&serde_json::json!({
                "project": project,
                "enrolled": !remove,
                "changed": changed,
                "projects": config.projects,
            }))?;
        }
        // `Uninstall` belongs here for a reason beyond tidiness: the match
        // below runs *after* `Store::open`, which creates the database when it
        // is missing. Handled there, an uninstall would delete the store and
        // then put an empty one back in the same run.
        Command::Setup { .. }
        | Command::Cloud { .. }
        | Command::CurrentProject
        | Command::Uninstall { .. } => {
            unreachable!("stateless command handled before opening the store")
        }
    }
    Ok(())
}

/// Says which of its two reasons an empty answer has, on stderr.
///
/// A read the directory narrowed can come back empty because the store has
/// never heard of this, or because it is filed under another project, and the
/// two call for opposite actions. `leteo search` has said which since the CLI
/// reads were scoped; `recent` and `context` printed `[]` and `""` and left
/// somebody to work it out — the same silence `mem_search`, `mem_context` and
/// the session-start block each had, found one surface at a time.
///
/// Written once here rather than three times, and paid only on an empty answer
/// with a project nobody named: an explicit `--project` is somebody who knows
/// where they are looking, and `--all-projects` has already looked everywhere.
fn say_where_it_is_empty(store: &Store, scope: &ReadScope) {
    if !scope.inferred {
        return;
    }
    let Some(project) = scope.project.as_deref() else {
        return;
    };
    let Ok(elsewhere) = store.memories_outside(project, crate::mcp::ELSEWHERE_CAP) else {
        return;
    };
    if elsewhere > 0 {
        eprintln!(
            "leteo: {}",
            crate::mcp::no_match_here_hint(
                project,
                elsewhere as usize,
                crate::mcp::ELSEWHERE_CAP,
                "--all-projects",
            )
        );
    }
}

async fn run_conflicts(store: &mut Store, command: ConflictsCommand) -> Result<()> {
    match command {
        ConflictsCommand::List {
            project,
            status,
            since,
            limit,
            offset,
        } => {
            let project = resolve_project(project)?;
            let options = ListRelationsOptions {
                project: Some(project.clone()),
                status,
                since,
                limit: Some(limit),
                offset,
            };
            let total = store.count_relations(options.clone())?;
            let relations = store.list_relations(options)?;
            print_json(&serde_json::json!({
                "project": project,
                "total": total,
                "showing": relations.len(),
                "relations": relations,
            }))
        }
        ConflictsCommand::Show { id } => {
            // The list item carries the observation titles; the relation row
            // carries the verdict itself. A judgment is useless without its
            // reason, confidence, and author, so show both.
            let item = store.get_relation_by_id(id)?;
            let relation = store.get_relation(&item.sync_id)?;
            print_json(&serde_json::json!({
                "id": item.id,
                "sync_id": item.sync_id,
                "relation": relation.relation,
                "judgment_status": relation.judgment_status,
                "source_id": item.source_id,
                "source_title": item.source_title,
                "target_id": item.target_id,
                "target_title": item.target_title,
                "reason": relation.reason,
                "evidence": relation.evidence,
                "confidence": relation.confidence,
                "marked_by_actor": relation.marked_by_actor,
                "marked_by_kind": relation.marked_by_kind,
                "marked_by_model": relation.marked_by_model,
                "session_id": relation.session_id,
                "created_at": relation.created_at,
                "updated_at": relation.updated_at,
            }))
        }
        ConflictsCommand::Stats { project } => {
            print_json(&store.relation_stats(Some(&resolve_project(project)?))?)
        }
        ConflictsCommand::Scan {
            project,
            since,
            dry_run: _,
            apply,
            max_insert,
            semantic,
            max_semantic,
            concurrency,
            timeout_per_call,
            yes,
        } => {
            let project = resolve_project(project)?;
            let Some(semantic) = semantic.filter(|value| !value.trim().is_empty()) else {
                return print_json(&store.scan_project(ScanOptions {
                    project,
                    since,
                    apply,
                    max_insert: Some(max_insert),
                })?);
            };
            let runner = crate::llm::Runner::parse(&semantic)?;
            let options = crate::llm::SemanticOptions {
                project,
                max_pairs: max_semantic,
                concurrency,
                timeout: std::time::Duration::from_secs(timeout_per_call),
            };
            if !yes {
                // Every pair is a paid model call, so the count is reported
                // first and nothing runs until the user opts in.
                let (pairs, inspected, already_judged) =
                    crate::llm::collect_pairs(store, &options)?;
                return print_json(&serde_json::json!({
                    "project": options.project,
                    "runner": runner.program(),
                    "inspected": inspected,
                    "pairs": pairs.len(),
                    // Said here for the reason `scan_project` says
                    // `already_related`: a preview whose numbers do not
                    // describe what the run will do is not a preview.
                    "already_judged": already_judged,
                    "dry_run": true,
                    "note": format!(
                        "judging these pairs makes up to {} {} call{}; re-run with --yes",
                        pairs.len(),
                        runner.program(),
                        if pairs.len() == 1 { "" } else { "s" }
                    ),
                }));
            }
            let timeout = options.timeout;
            let summary = crate::llm::semantic_scan(store, &options, |prompt| {
                crate::llm::cli_compare(runner, timeout, prompt)
            })
            .await?;
            print_json(&summary)
        }
        ConflictsCommand::Deferred {
            status,
            limit,
            inspect,
            replay,
        } => {
            if let Some(sync_id) = inspect {
                return print_json(&store.get_deferred(&sync_id)?);
            }
            if replay {
                let result = store.replay_deferred_sync_mutations()?;
                let (deferred, dead) = store.deferred_sync_counts()?;
                return print_json(&serde_json::json!({
                    "retried": result.retried,
                    "succeeded": result.succeeded,
                    "failed": result.failed,
                    "dead": result.dead,
                    "deferred_remaining": deferred,
                    "dead_total": dead,
                }));
            }
            let rows = store.list_deferred(ListDeferredOptions {
                status,
                limit: Some(limit),
                offset: 0,
            })?;
            print_json(&rows)
        }
    }
}

/// Runs a server-side administration command against the cloud database.
///
/// These commands operate on PostgreSQL directly, like `cloud serve`, so they
/// read the same environment configuration and never touch the local store.
async fn run_cloud_admin(command: &CloudAdminCommand) -> Result<()> {
    let config = crate::cloud::CloudConfig::from_env();
    if config.database_url.trim().is_empty() {
        anyhow::bail!("LETEO_DATABASE_URL is required for cloud administration");
    }
    let store = crate::cloud::CloudStore::connect(&config.database_url, config.max_pool).await?;
    store.migrate().await?;

    match command {
        CloudAdminCommand::Bootstrap {
            name,
            email,
            environment,
            project,
        } => {
            let hasher = crate::cloud::ManagedTokenHasher::new(&config.token_pepper)?;
            let principal_id = store.create_principal("human", name, "admin").await?;
            store
                .create_user(principal_id, name, email.as_deref())
                .await?;
            let token = crate::cloud::ManagedToken::generate(environment);
            let verifier = hasher.hash(&token.raw)?;
            let token_id = store
                .store_managed_token(principal_id, &token, &verifier, "bootstrap")
                .await?;
            for project in project {
                store.grant_project(principal_id, project).await?;
            }
            print_json(&serde_json::json!({
                "principal_id": principal_id,
                "token_id": token_id,
                "token": token.raw,
                "token_prefix": token.prefix,
                "projects": project,
                "warning": "store this token now; it is never recoverable",
            }))
        }
        CloudAdminCommand::Token {
            principal,
            environment,
            label,
        } => {
            let hasher = crate::cloud::ManagedTokenHasher::new(&config.token_pepper)?;
            let principal_id = resolve_principal(&store, principal).await?;
            let token = crate::cloud::ManagedToken::generate(environment);
            let verifier = hasher.hash(&token.raw)?;
            let token_id = store
                .store_managed_token(principal_id, &token, &verifier, label)
                .await?;
            print_json(&serde_json::json!({
                "principal_id": principal_id,
                "token_id": token_id,
                "token": token.raw,
                "token_prefix": token.prefix,
                "warning": "store this token now; it is never recoverable",
            }))
        }
        CloudAdminCommand::Grant {
            principal,
            project,
            revoke,
        } => {
            let principal_id = resolve_principal(&store, principal).await?;
            let changed = if *revoke {
                store.revoke_project_grant(principal_id, project).await?
            } else {
                store.grant_project(principal_id, project).await?;
                true
            };
            print_json(&serde_json::json!({
                "principal_id": principal_id,
                "project": project,
                "granted": !revoke,
                "changed": changed,
                "grants": store.list_principal_project_grants(principal_id).await?,
            }))
        }
        CloudAdminCommand::ProjectSync {
            project,
            enable,
            disable,
        } => {
            if !enable && !disable {
                anyhow::bail!("pass --enable or --disable");
            }
            let enabled = *enable;
            store
                .set_project_sync_enabled(
                    project,
                    enabled,
                    &crate::sync::created_by(),
                    Some("leteo cloud admin project-sync"),
                )
                .await?;
            print_json(&serde_json::json!({
                "project": project,
                "sync_enabled": enabled,
            }))
        }
        CloudAdminCommand::Status => {
            store.health().await?;
            print_json(&serde_json::json!({
                "healthy": true,
                "stats": store.stats().await?,
            }))
        }
    }
}

/// Accepts a numeric principal identifier or a display name.
async fn resolve_principal(store: &crate::cloud::CloudStore, principal: &str) -> Result<i64> {
    if let Ok(id) = principal.trim().parse::<i64>() {
        return Ok(id);
    }
    store
        .find_principal_by_name(principal)
        .await?
        .with_context(|| format!("no principal named {principal:?}"))
}

/// Makes the store's enrolment say exactly what the config's project list says.
///
/// The two are read at opposite ends of replication: the list in `cloud.json`
/// decides what the background loop is allowed to push, and
/// `sync_enrolled_projects` decides what is journalled to be pushed at all. A
/// project in one and not the other is a replication that reports itself
/// healthy and moves nothing — or, the other way round, a journal that fills up
/// for a project the loop will never send.
fn reconcile_enrolment(store: &mut crate::Store, projects: &[String]) -> anyhow::Result<()> {
    let wanted: std::collections::BTreeSet<String> = projects
        .iter()
        .map(|project| crate::memory::normalize::project(project))
        .filter(|project| !project.is_empty())
        .collect();
    for project in store.enrolled_projects()? {
        if !wanted.contains(&project) {
            store.unenroll_project(&project)?;
        }
    }
    for project in wanted {
        store.enroll_project(&project)?;
    }
    Ok(())
}
