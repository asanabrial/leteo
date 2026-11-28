use super::*;

#[derive(Debug, Parser)]
#[command(
    name = "leteo",
    version = crate::build_info::version(),
    about = "Persistent memory for AI coding agents"
)]
pub struct Cli {
    /// Directory holding the store. Falls back to $LETEO_DATA_DIR.
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
    /// Path of the database file. Falls back to $LETEO_DATABASE.
    #[arg(long, global = true)]
    pub database: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Open a memory session.
    SessionStart {
        id: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        directory: PathBuf,
    },
    /// Close a session and record its summary.
    SessionEnd {
        id: String,
        #[arg(long)]
        summary: Option<String>,
    },
    /// Save a memory.
    Save {
        title: String,
        content: String,
        #[arg(long, default_value = "manual")]
        r#type: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "project")]
        scope: String,
        #[arg(long)]
        topic_key: Option<String>,
        #[arg(long)]
        tool_name: Option<String>,
    },
    /// Replace several memories with one, recording a judged `supersedes`
    /// relation from the new memory to each source so the merge is traceable
    /// and reversible.
    Consolidate {
        /// Observation ids to replace.
        #[arg(required = true, num_args = 1..)]
        source_ids: Vec<i64>,
        #[arg(long, default_value = "manual")]
        r#type: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        content: String,
        #[arg(long, default_value = "project")]
        scope: String,
        #[arg(long)]
        topic_key: Option<String>,
    },
    /// Search memories by text.
    Search {
        query: String,
        #[arg(long)]
        project: Option<String>,
        /// Search every project instead of the one this directory belongs to.
        #[arg(long, conflicts_with = "project")]
        all_projects: bool,
        #[arg(long)]
        r#type: Option<String>,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long, value_enum, default_value_t = MatchMode::All)]
        match_mode: MatchMode,
    },
    /// Record a user prompt.
    Prompt {
        content: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        project: Option<String>,
    },
    /// List the most recent memories.
    Recent {
        #[arg(long)]
        project: Option<String>,
        /// List from every project instead of the one this directory belongs to.
        #[arg(long, conflicts_with = "project")]
        all_projects: bool,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Include the session summaries, which are left out by default.
        #[arg(long)]
        summaries: bool,
    },
    /// Remove a memory, a session, a prompt or a whole project.
    Delete {
        #[command(subcommand)]
        command: DeleteCommand,
    },
    /// Inspect and maintain the projects the store holds.
    Projects {
        #[command(subcommand)]
        command: ProjectsCommand,
    },
    /// Show the memories around one observation in time.
    Timeline {
        id: i64,
        #[arg(long)]
        before: Option<usize>,
        #[arg(long)]
        after: Option<usize>,
    },
    /// Print the context a session opens with.
    Context {
        /// Project to build the context for. `--project` is accepted too.
        ///
        /// Every other command that takes a project takes it as `--project`,
        /// and this one alone took it as a position. Reaching for the flag out
        /// of habit got "unexpected argument '--project'" — so both work, and
        /// nobody's script breaks.
        project: Option<String>,
        #[arg(long = "project", value_name = "PROJECT", conflicts_with = "project")]
        project_flag: Option<String>,
        /// Build the context from every project instead of the one this
        /// directory belongs to.
        #[arg(long, conflicts_with_all = ["project", "project_flag"])]
        all_projects: bool,
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Check the store and report what is wrong with it.
    Doctor {
        /// Report only this diagnostic, by its stable code.
        #[arg(long)]
        check: Option<String>,
        /// Scope the report to one project.
        #[arg(long)]
        project: Option<String>,
        /// Rebuild the full-text indexes before reporting.
        ///
        /// For the break the report can already see and nobody could fix: an
        /// index that has gone empty or fallen behind its table answers every
        /// search with nothing.
        #[arg(long)]
        repair: bool,
        /// Also list memories whose cited commits or paths no longer resolve.
        ///
        /// Opt-in because it runs `git` and reads the working tree of the
        /// repository this directory is in, and reports only the project that
        /// repository is. It is a warning and never makes the store unhealthy;
        /// nothing is hidden, marked or written. `--check stale_citations`
        /// asks for it alone.
        #[arg(long, conflicts_with = "check")]
        citations: bool,
    },
    /// Find and judge conflicting memories.
    Conflicts {
        #[command(subcommand)]
        command: ConflictsCommand,
    },
    /// Export memories as JSON.
    Export {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Import memories from a JSON export or from an Engram install.
    Import {
        /// JSON export to read. `--input` is accepted too. Omit when using
        /// --from-engram.
        ///
        /// `export` writes with `--output`, so reaching for `--input` here is
        /// the natural symmetry, and it used to be an error.
        file: Option<PathBuf>,
        #[arg(long = "input", value_name = "FILE", conflicts_with = "file")]
        input: Option<PathBuf>,
        /// Take over an existing Engram installation's memories instead.
        ///
        /// A file names the other source, so the two are refused together
        /// rather than one of them silently winning.
        #[arg(long, conflicts_with_all = ["file", "input"])]
        from_engram: bool,
        /// The Engram database to adopt. Defaults to ~/.engram/engram.db.
        ///
        /// Only `--from-engram` reads it, so naming it alone is refused rather
        /// than accepted as a path nothing looks at.
        #[arg(long, requires = "from_engram")]
        source: Option<PathBuf>,
        /// Report what would be imported without writing the rows.
        #[arg(long)]
        dry_run: bool,
    },
    /// Export memories into an Obsidian vault as linked Markdown notes.
    ObsidianExport {
        #[arg(long)]
        vault: PathBuf,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, value_enum, default_value_t = GraphConfigArgument::Preserve)]
        graph_config: GraphConfigArgument,
    },
    /// Report store counts.
    Stats,
    /// Serve the store over a local HTTP API, and keep replicating in the
    /// background while it runs.
    ///
    /// This is the port the command was named for. It was removed when nothing
    /// used it — the hooks, the MCP server, the CLI and the TUI all reach
    /// SQLite directly — and is back for the callers that already speak HTTP:
    /// a script reaches `POST /tools/<name>` with the arguments the MCP tool of
    /// that name takes, so there is one name and one argument shape per
    /// operation. The loopback is the default; `--bind` takes another address,
    /// and one that is not the loopback is an explicit opt-in the log says out
    /// loud.
    Serve {
        /// Address to listen on, as `host:port`.
        #[arg(long, default_value = crate::mcp::DEFAULT_BIND)]
        bind: String,
    },
    /// Serve the MCP tools an agent calls.
    Mcp {
        /// Comma-separated tool profiles or names: agent, admin, all, or
        /// individual tools such as mem_save.
        #[arg(long, env = "LETEO_TOOLS")]
        tools: Option<String>,
        /// Trusted project for this process, applied before directory
        /// detection when a write has no session.
        #[arg(long, env = "LETEO_PROJECT")]
        project: Option<String>,
    },
    /// Open the interactive terminal interface.
    Tui,
    /// Handle an agent lifecycle event. Reads the agent's JSON payload from
    /// standard input and prints the hook response on standard output.
    Hook {
        event: HookEventArgument,
        /// Print the full outcome instead of the agent hook response.
        #[arg(long)]
        verbose: bool,
    },
    /// Configure Leteo in an agent.
    Setup {
        agent: Option<String>,
        /// Walk through setup even when the output is not a terminal.
        #[arg(long)]
        interactive: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        instructions: bool,
        /// Install the lifecycle hooks that keep memory automatic.
        #[arg(long)]
        hooks: bool,
        /// Tool profile the configured server exposes: agent, admin, all, or a
        /// comma-separated list of tool names. Defaults to agent.
        #[arg(long)]
        tools: Option<String>,
        /// The language memories are written in, saved for every agent.
        ///
        /// Free text, because it is handed to a model rather than parsed:
        /// `español`, `Spanish`, `português do Brasil`. Omit it and each memory
        /// is written in the language of the conversation it came from, which
        /// is the default and is not what an agent does if nobody says so.
        #[arg(long)]
        language: Option<String>,
        /// How many memories a session opens with: `slim`, `full` or `deep`.
        ///
        /// Twenty, thirty or eighty. What each buys is measured, and the table
        /// is on `settings::ContextSize`: the first twenty are worth 4.9 points
        /// of recall a kilobyte and the last thirty 0.75. `slim` is for a small
        /// context window, `deep` for a store that matters more than the
        /// budget. Like `--language`, this alone is a complete command.
        #[arg(long)]
        context: Option<String>,
        /// Take Leteo out of this agent instead of putting it in.
        ///
        /// Removes the MCP entry, the lifecycle hooks and the protocol block,
        /// and nothing else: other servers, other tools' hooks and anybody's
        /// own notes are left where they are.
        #[arg(long, conflicts_with_all = ["instructions", "hooks", "tools"])]
        uninstall: bool,
    },
    /// Remove Leteo from this machine entirely.
    ///
    /// Every agent it was configured in, the store, the settings, and — where
    /// the operating system permits it — the binary. `setup --uninstall` takes
    /// Leteo out of one agent and leaves the memories alone; this leaves
    /// nothing.
    ///
    /// A `.leteo/config.json` inside a repository is not touched. It names the
    /// project that checkout belongs to, so it
    /// belongs to that checkout rather than to this install.
    Uninstall {
        /// Carry it out. Without this, the command reports what would go and
        /// changes nothing.
        ///
        /// Opt-in rather than a confirmation prompt: this command is what the
        /// installers and the Windows uninstaller call, and a question asked
        /// where no console is attached is a hang rather than a safeguard.
        #[arg(long)]
        yes: bool,
        /// Keep the store. Memories are the one thing Leteo cannot fetch again,
        /// and a `uninstall --yes` that takes them is not undoable; this leaves
        /// the data directory — the store, its settings and any `pre-schema`
        /// migration copy — where it is.
        #[arg(long)]
        keep_data: bool,
    },
    /// Replicate memories to a cloud server.
    Cloud {
        #[command(subcommand)]
        command: CloudCommand,
    },
    /// Print the project this directory belongs to.
    CurrentProject,
    /// The model semantic search reads, which ships beside the binary.
    Model {
        #[command(subcommand)]
        action: ModelCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    /// Put the model where Leteo looks for it, checked against the hashes this
    /// build was made with.
    ///
    /// For an install that arrived without the model: `cargo install`, a build
    /// from source, a package that left it out. An install that came with the
    /// model beside it already has it. Downloads the files committed at the tag
    /// that matches this binary, unless `--from` names a directory to copy from.
    /// Nothing is written unless every file verifies, and what was installed
    /// before is left as it was otherwise.
    Install {
        /// A directory holding config.json, model.safetensors and
        /// tokenizer.json.gz: a `model/` directory from a copy of the files, for a
        /// machine with no network.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Fetch from this directory of the three files, under their own names,
        /// instead of the files committed at this version's tag (also
        /// `LETEO_MODEL_URL`).
        #[arg(long, conflicts_with = "from")]
        url: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum DeleteCommand {
    /// Delete one memory.
    Observation {
        id: i64,
        #[arg(long)]
        hard: bool,
    },
    /// Delete a session and the memories it recorded.
    Session { id: String },
    /// Delete one prompt.
    Prompt { id: i64 },
    /// Delete every memory in a project.
    Project {
        name: String,
        #[arg(long)]
        hard: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ProjectsCommand {
    /// List the projects the store holds.
    List,
    /// Merge duplicate projects into one.
    Consolidate {
        #[arg(long, conflicts_with = "all")]
        project: Option<String>,
        #[arg(long, conflicts_with = "project")]
        all: bool,
        #[arg(long)]
        apply: bool,
    },
    /// Remove projects that hold no memories.
    Prune {
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConflictsCommand {
    /// List the conflicts found for a project.
    List {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
    },
    /// Show one conflict and the memories it relates.
    Show { id: i64 },
    /// Report conflict counts.
    Stats {
        #[arg(long)]
        project: Option<String>,
    },
    /// Look for conflicts the store does not know about yet.
    Scan {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long, conflicts_with = "apply")]
        dry_run: bool,
        #[arg(long, conflicts_with = "dry_run")]
        apply: bool,
        #[arg(long, default_value_t = 100)]
        max_insert: usize,
        /// Judge candidate pairs with an agent CLI instead of only listing them.
        /// Every pair costs one model call.
        #[arg(long, env = "LETEO_AGENT_CLI")]
        semantic: Option<String>,
        #[arg(long, default_value_t = 100)]
        max_semantic: usize,
        #[arg(long, default_value_t = 5)]
        concurrency: usize,
        #[arg(long, default_value_t = 60)]
        timeout_per_call: u64,
        /// Judge the pairs instead of only reporting how many there are.
        #[arg(long)]
        yes: bool,
    },
    /// List the sync mutations that could not be applied, or retry them.
    Deferred {
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        #[arg(long, conflicts_with = "replay")]
        inspect: Option<String>,
        #[arg(long, conflicts_with = "inspect")]
        replay: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum CloudCommand {
    /// Run the cloud server.
    #[cfg(feature = "cloud-server")]
    Serve,
    /// Check whether the configured cloud server answers.
    Health {
        #[arg(long, env = "LETEO_CLOUD_SERVER")]
        server: Option<String>,
        #[arg(long, env = "LETEO_CLOUD_TOKEN")]
        token: Option<String>,
    },
    /// Replicate this machine's memories to the cloud once.
    Sync {
        #[arg(long, env = "LETEO_CLOUD_SERVER")]
        server: Option<String>,
        #[arg(long, env = "LETEO_CLOUD_TOKEN")]
        token: Option<String>,
        #[arg(long)]
        project: Option<String>,
    },
    /// Inspect or change the persisted client configuration.
    Config {
        #[command(subcommand)]
        command: CloudConfigCommand,
    },
    /// Report local replication state without contacting the server.
    Status,
    /// Enroll a project for cloud replication.
    Enroll {
        #[arg(long)]
        project: Option<String>,
        /// Remove the project from replication instead.
        #[arg(long)]
        remove: bool,
    },
    /// Server-side administration against the cloud PostgreSQL database.
    #[cfg(feature = "cloud-server")]
    Admin {
        #[command(subcommand)]
        command: CloudAdminCommand,
    },
}

#[cfg(feature = "cloud-server")]
#[derive(Debug, Subcommand)]
pub enum CloudAdminCommand {
    /// Create the first administrator and print its managed token once.
    Bootstrap {
        #[arg(long)]
        name: String,
        #[arg(long)]
        email: Option<String>,
        #[arg(long, default_value = "prod")]
        environment: String,
        /// Projects the administrator may sync; `*` grants every project.
        #[arg(long, default_value = "*")]
        project: Vec<String>,
    },
    /// Mint an additional managed token for an existing principal.
    Token {
        #[arg(long)]
        principal: String,
        #[arg(long, default_value = "prod")]
        environment: String,
        #[arg(long, default_value = "managed token")]
        label: String,
    },
    /// Grant or revoke a principal's access to a project.
    Grant {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        revoke: bool,
    },
    /// Revoke a managed token, so every request carrying it is refused.
    Revoke {
        /// Token id, the prefix printed when it was minted, or the token.
        #[arg(long)]
        token: String,
        /// Why it is being revoked, kept with the row.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Disable a principal, refusing every managed token it holds.
    Disable {
        /// Principal id, or its display name.
        #[arg(long)]
        principal: String,
    },
    /// Re-enable a principal, and with it the tokens it still holds.
    Enable {
        /// Principal id, or its display name.
        #[arg(long)]
        principal: String,
    },
    /// Pause or resume replication for a project across the whole service.
    ProjectSync {
        #[arg(long)]
        project: String,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
    },
    /// Report cloud database health and aggregate counts.
    Status,
}

#[derive(Debug, Subcommand)]
pub enum CloudConfigCommand {
    /// Print the effective configuration with the token redacted.
    Show,
    /// Change the persisted client configuration.
    Set {
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        token: Option<String>,
        /// Replace the replicated project list.
        #[arg(long)]
        project: Vec<String>,
        #[arg(long)]
        poll_interval: Option<u64>,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
    },
    /// Delete the persisted configuration file.
    Clear,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum GraphConfigArgument {
    /// Write the Leteo graph defaults only when the vault has none.
    Preserve,
    /// Overwrite the vault's graph settings.
    Force,
    /// Leave the vault's graph settings untouched.
    Skip,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum HookEventArgument {
    SessionStart,
    PostCompaction,
    UserPromptSubmit,
    SubagentStop,
    SessionStop,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum MatchMode {
    All,
    Any,
}

impl Cli {
    /// Whether this command is worth a thread per core.
    ///
    /// `#[tokio::main]` builds the multi-threaded scheduler, which spawns one
    /// worker per core before `main` runs a line: measured on a sixteen-core
    /// machine, 5.39 ms against 4.50 for the same binary on the current-thread
    /// scheduler, where a bare `fn main` costs 4.51. Nine tenths of a
    /// millisecond, on every hook, on every prompt somebody types.
    ///
    /// The release profile already carries the other half of this
    /// investigation: taking 3.2 MB off the binary moved start by nothing, so
    /// what is worth attacking is what runs before `main` rather than the size
    /// of the image. This is that.
    ///
    /// Threads go to what stays running — a server, a background replication
    /// loop, a screen — and everything else is one shot: parse, open the
    /// store, answer, exit. Those never had anything to schedule in parallel.
    pub fn wants_worker_threads(&self) -> bool {
        matches!(
            self.command,
            Command::Serve { .. } | Command::Mcp { .. } | Command::Tui | Command::Cloud { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("leteo").chain(arguments.iter().copied()))
    }

    #[test]
    fn a_project_reaches_context_by_position_or_by_flag() {
        // Every other command that takes a project takes `--project`. This one
        // alone took a position, so reaching for the flag out of habit was an
        // "unexpected argument" — including for whoever wrote this.
        let by_position = parse(&["context", "leteo"]).unwrap();
        let by_flag = parse(&["context", "--project", "leteo"]).unwrap();

        for parsed in [by_position, by_flag] {
            let Command::Context {
                project,
                project_flag,
                ..
            } = parsed.command
            else {
                panic!("expected the context command");
            };
            assert_eq!(project_flag.or(project).as_deref(), Some("leteo"));
        }
    }

    #[test]
    fn a_file_reaches_import_by_position_or_as_input() {
        // `export` writes with `--output`, so `--input` is the symmetry
        // somebody will reach for.
        let by_position = parse(&["import", "dump.json"]).unwrap();
        let by_flag = parse(&["import", "--input", "dump.json"]).unwrap();

        for parsed in [by_position, by_flag] {
            let Command::Import { file, input, .. } = parsed.command else {
                panic!("expected the import command");
            };
            assert_eq!(input.or(file).unwrap(), PathBuf::from("dump.json"));
        }
    }

    #[test]
    fn saying_it_both_ways_at_once_is_refused_rather_than_silently_picking_one() {
        assert!(parse(&["context", "leteo", "--project", "other"]).is_err());
        assert!(parse(&["import", "a.json", "--input", "b.json"]).is_err());
    }

    #[test]
    fn an_engram_import_refuses_a_file_and_a_source_without_the_flag() {
        // `--from-engram` adopts a database; a file names a different source,
        // so one of the two would be read as nothing.
        assert!(parse(&["import", "dump.json", "--from-engram"]).is_err());
        assert!(parse(&["import", "--input", "dump.json", "--from-engram"]).is_err());
        // `--source` names the Engram database and nothing else reads it.
        assert!(parse(&["import", "--source", "engram.db"]).is_err());
        assert!(parse(&["import", "--from-engram", "--source", "engram.db"]).is_ok());
    }

    #[test]
    fn the_command_tree_is_internally_consistent() {
        // clap's own audit: duplicate argument ids, a `conflicts_with` naming
        // an argument that does not exist, and similar wiring mistakes are
        // debug-time panics rather than compile errors.
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn every_subcommand_says_what_it_does() {
        // `about` is the line `--help` prints beside a command, and clap leaves
        // it empty when the variant carries no doc comment: the command is then
        // listed with nothing to say for itself. That blank is invisible in a
        // diff and obvious here, which is the whole point of walking the tree
        // rather than trusting a reader to notice one missing `///`.
        use clap::CommandFactory;

        fn collect(command: &clap::Command, path: &str, blank: &mut Vec<String>) {
            for subcommand in command.get_subcommands() {
                let here = if path.is_empty() {
                    subcommand.get_name().to_owned()
                } else {
                    format!("{path} {}", subcommand.get_name())
                };
                if subcommand
                    .get_about()
                    .is_none_or(|about| about.to_string().trim().is_empty())
                {
                    blank.push(here.clone());
                }
                collect(subcommand, &here, blank);
            }
        }

        let mut blank = Vec::new();
        collect(&Cli::command(), "", &mut blank);
        assert!(
            blank.is_empty(),
            "these subcommands have no `about`, so `--help` shows them bare: {blank:?}"
        );
    }

    #[test]
    fn uninstall_takes_the_yes_and_the_keep_data_flags() {
        // The two flags `uninstall` carries: `--yes` carries the removal out,
        // and `--keep-data` leaves the store and its migration copies where
        // they are. Both reach `SetupOptions` in `cli/mod.rs`.
        let parsed = parse(&["uninstall", "--yes", "--keep-data"]).unwrap();
        let Command::Uninstall { yes, keep_data } = parsed.command else {
            panic!("expected the uninstall command");
        };
        assert!(yes, "the flag that carries the removal out");
        assert!(keep_data, "and the one that keeps the store out of it");
    }
}

#[cfg(test)]
mod runtime_choice {
    use super::*;
    use clap::Parser;

    /// Only what keeps running gets a thread per core.
    ///
    /// The multi-threaded scheduler spawns one worker per core before `main`
    /// runs a line — measured at 5.39 ms against 4.50 for the current-thread
    /// one on a sixteen-core machine, where a bare `fn main` is 4.51. Every
    /// hook and every prompt somebody types pays that, and a hook has nothing
    /// to schedule in parallel.
    #[test]
    fn one_shot_commands_do_not_pay_for_a_thread_per_core() {
        let quiere = |args: &[&str]| Cli::parse_from(args).wants_worker_threads();

        // The hot ones: a hook fires on every prompt, and the rest are one
        // shot — parse, open the store, answer, exit.
        assert!(!quiere(&["leteo", "hook", "session-start"]));
        assert!(!quiere(&["leteo", "hook", "user-prompt-submit"]));
        assert!(!quiere(&["leteo", "search", "algo"]));
        assert!(!quiere(&["leteo", "current-project"]));
        assert!(!quiere(&["leteo", "stats"]));
        assert!(!quiere(&["leteo", "doctor"]));

        // And what stays running, which is where a scheduler earns its threads:
        // both of these start the background replication loop.
        assert!(quiere(&["leteo", "serve"]));
        assert!(quiere(&["leteo", "mcp"]));
        assert!(quiere(&["leteo", "tui"]));
        assert!(quiere(&["leteo", "cloud", "status"]));
    }
}
