use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::{Map, Value, json};

mod agents;
mod removal;
mod render;
pub mod wizard;

use render::*;

pub const SERVER_NAME: &str = "leteo";
pub const MEMORY_PROTOCOL_BEGIN: &str =
    "<!-- BEGIN LETEO MEMORY PROTOCOL - managed by leteo setup -->";
pub const MEMORY_PROTOCOL_END: &str = "<!-- END LETEO MEMORY PROTOCOL -->";

pub const MEMORY_DIRECTIVE: &str = r#"## Leteo memory — active

You have persistent memory through Leteo's MCP tools. Three rules:

1. **Save as you go.** Call `mem_save` immediately after a decision, a bug fix,
   a non-obvious discovery, a convention, or a preference the user states. Do
   not wait to be asked, and do not batch it to the end.
2. **Saving is not replying.** A memory is written for your future self; the
   user never sees it. Never end a turn with a `mem_save` where the answer
   should have been, and never let a failed save swallow the reply.
3. **Recall before assuming.** When past work might be relevant, call
   `mem_search`, then `mem_get_observation` for the whole text of anything that
   looks right. `mem_context` is for a whole project, and repeats the block a
   session opens with.

The `leteo-memory` skill has the rest: how to word a memory, which project it
belongs to, how to judge a conflict, and how to close a session."#;

pub const MEMORY_PROTOCOL: &str = r#"## Leteo Persistent Memory - Protocol

Leteo is persistent memory for coding agents. Its MCP tools survive sessions and context compaction.

### Save important work

Call `mem_save` immediately after completing a bug fix, making an architecture or design decision,
discovering a non-obvious constraint, changing configuration, or establishing a reusable convention.
Use a short searchable title and structure the content as What, Why, Where, and Learned.

Write dense rather than short. Spend nothing on filler, hedging or repeating the title, and nothing
on what the repository already records. Spend freely on names, paths, numbers, and error strings
quoted verbatim: those are what a later search matches on and what makes the memory worth keeping.
Full sentences with their articles — a person reads these in the TUI as well as an agent.

### Recall before acting

When prior work may be relevant, call `mem_search` with the words of the thing you are asking
about, then `mem_get_observation` when the complete observation is needed.

`mem_context` answers a different question — everything a project holds — and it is what a session
opens with where Leteo installs hooks. Measured on a real store, that opening block named fifty
memories in 11 KB and `mem_context` came back with twenty of those same fifty in 22 KB. So call it
when the session did not open with one, when you have lost it, or for another project; not as the
first step of looking something up.

### Close sessions

Before ending substantial work, call `mem_session_summary` with the goal, discoveries,
accomplishments, next steps, and relevant files. Open with what the session was for: the first line
that is not a heading becomes the memory's title, and a summary that starts with a date is one
nobody can find again. After context compaction, persist the compacted summary first and then
restore context with `mem_context`.

### Report memory work as Sardi

Leteo is the store; Sardi is the cat who tends it. When you tell the user what became of their
memories, say who did it: "Sardi kept that one", "Sardi remembers three notes about this from
March", "Sardi merged it with the earlier decision". Use your own wording - this is a register,
not a script.

Keep it to once per reply, and only when there is something worth reporting; a mascot that
narrates every tool call is noise. Never put it in an error: a failure has to stay precise and
actionable, and there is nothing charming about a cat standing between someone and the thing they
need to fix."#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Unix,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Unix
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpFormat {
    McpServers,
    Servers,
    Mcp,
    Pi,
    /// ZCode reads `mcp.servers` inside `~/.zcode/cli/config.json` — a path two
    /// keys deep, unlike every other format here. Verified in the client's own
    /// source, where the user scope resolves to `.zcode/cli` + `config.json`
    /// with `mcp.servers` as its key (`userConfigDirSegments: [".zcode",
    /// "cli"]`), and where a scope holding no servers falls back to
    /// `.agents/mcp.json`. Writing the fallback file instead would demote
    /// Leteo silently the moment a native server appeared, so the native key
    /// gets the entry.
    Zcode,
}

impl McpFormat {
    fn key_path(self) -> &'static [&'static str] {
        match self {
            Self::McpServers | Self::Pi => &["mcpServers"],
            Self::Servers => &["servers"],
            Self::Mcp => &["mcp"],
            Self::Zcode => &["mcp", "servers"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Json(McpFormat),
    CodexToml,
    DshPatch,
}

pub use agents::AgentAdapter;
pub use removal::{AgentRemoval, Removal, uninstall_everything};

pub fn supported_agents() -> &'static [AgentAdapter] {
    agents::REGISTRY
}

#[derive(Debug, Clone, Default)]
pub struct SetupOptions {
    pub dry_run: bool,
    pub install_instructions: bool,
    pub install_hooks: bool,
    pub tools: Option<String>,
    pub platform: Option<Platform>,
    pub home_dir: Option<PathBuf>,
    pub config_home: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
    pub executable: Option<PathBuf>,
    pub dsh_home: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentPaths {
    pub mcp_config: PathBuf,
    pub instructions: Option<PathBuf>,
    pub hooks: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    McpConfiguration,
    Instructions,
    Hooks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Create,
    Update,
    Unchanged,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupAction {
    pub kind: ActionKind,
    pub path: PathBuf,
    pub change: Change,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept_for: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupResult {
    pub agent: &'static str,
    pub dry_run: bool,
    pub actions: Vec<SetupAction>,
}

impl SetupResult {
    pub fn changed_files(&self) -> usize {
        self.actions
            .iter()
            .filter(|action| action.change != Change::Unchanged)
            .count()
    }
}

#[derive(Debug)]
struct SetupEnvironment {
    platform: Platform,
    home: PathBuf,
    config_home: Option<PathBuf>,
    app_data: Option<PathBuf>,
    executable: PathBuf,
    claude_config: Option<PathBuf>,
    dsh_home: Option<PathBuf>,
}

impl SetupEnvironment {
    fn claude_config_dir(&self) -> PathBuf {
        self.claude_config
            .clone()
            .unwrap_or_else(|| self.home.join(".claude"))
    }
}

pub fn resolve_agent_paths(agent: &str, options: &SetupOptions) -> Result<AgentPaths> {
    let adapter = find_adapter(agent)?;
    let environment = SetupEnvironment::resolve(options)?;
    Ok(resolve_paths(adapter, &environment))
}

pub fn is_configured(agent: &str, options: &SetupOptions) -> bool {
    let Ok(adapter) = find_adapter(agent) else {
        return false;
    };
    let Ok(environment) = SetupEnvironment::resolve(options) else {
        return false;
    };
    let path = resolve_paths(adapter, &environment).mcp_config;
    let Ok(text) = fs::read_to_string(&path) else {
        return false;
    };
    match adapter.config_format {
        ConfigFormat::CodexToml => text
            .lines()
            .any(|line| line.trim() == "[mcp_servers.leteo]"),
        ConfigFormat::DshPatch => dsh_names_leteo(&text),
        ConfigFormat::Json(format) => serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|config| {
                servers_at(&config, format).map(|servers| servers.contains_key(SERVER_NAME))
            })
            .unwrap_or(false),
    }
}

fn servers_at(config: &Value, format: McpFormat) -> Option<&Map<String, Value>> {
    let mut node = config;
    for key in format.key_path() {
        node = node.get(key)?;
    }
    node.as_object()
}

pub fn uninstall(agent: &str, options: &SetupOptions) -> Result<SetupResult> {
    let adapter = find_adapter(agent)?;
    let environment = SetupEnvironment::resolve(options)?;
    let paths = resolve_paths(adapter, &environment);
    let mut actions = Vec::new();

    if let Some(existing) = read_optional(&paths.mcp_config)? {
        let desired = match adapter.config_format {
            ConfigFormat::Json(format) => remove_json_server(&existing, format)?,
            ConfigFormat::CodexToml => remove_codex_server(&existing)?,
            ConfigFormat::DshPatch => remove_dsh_server(&existing)?,
        };
        actions.push(apply_content(
            &paths.mcp_config,
            Some(&existing),
            desired.as_bytes(),
            ActionKind::McpConfiguration,
            options.dry_run,
        )?);
    }

    if let Some(instructions) = paths.instructions.as_ref()
        && let Some(existing) = read_optional(instructions)?
    {
        let text = std::str::from_utf8(&existing)
            .with_context(|| format!("{} is not valid UTF-8", instructions.display()))?;
        let desired = remove_memory_protocol(text);
        // A file Leteo invented and nobody else wrote in goes, rather than
        // staying behind empty with Leteo's name on it — `uninstall` says it
        // removes Leteo from this machine entirely, and three agents get a file
        // of their own: `leteo-memory-protocol.md`, `leteo.md`,
        // `leteo.instructions.md`. Copilot's kept its `applyTo` front matter
        // too, which is an instruction file that applies to every source file
        // and says nothing.
        //
        // What is left over decides it, not the name. Somebody who added their
        // own paragraph to that file still has it: the leftover is only ever
        // removed when it is empty, or when it is exactly the preamble Leteo
        // wrote to create the file in the first place.
        //
        // Unless somebody else is still reading it. One instruction file can be
        // loaded by two products — the Gemini CLI takes `~/.gemini/GEMINI.md` as
        // its context file and Antigravity takes the same path as its global
        // memories, which is verified in each one's own bundle rather than
        // inferred from the directory. Splicing the block out for one of them
        // takes it from the other, which still has the MCP server and would go
        // on running with no instructions and nothing said about it.
        let leftover = desired.trim();
        let ours = adapter.owns_instruction_file
            && (leftover.is_empty() || leftover == adapter.new_instruction_file.trim());
        actions.push(
            match still_configured_sharer(adapter, instructions, &environment)? {
                Some(other) => SetupAction {
                    kind: ActionKind::Instructions,
                    path: instructions.clone(),
                    change: Change::Unchanged,
                    kept_for: Some(other),
                },
                None if ours => {
                    remove_file(instructions, ActionKind::Instructions, options.dry_run)?
                }
                None => apply_content(
                    instructions,
                    Some(&existing),
                    desired.as_bytes(),
                    ActionKind::Instructions,
                    options.dry_run,
                )?,
            },
        );
    }

    // Skipped where the hooks share the server's file, because taking the
    // server out already took them: `remove_codex_server` strips both. Running
    // again here would read the TOML that is left and fail trying to parse it
    // as the JSON settings file every other agent keeps hooks in.
    if let Some(hooks) = paths.hooks.as_ref()
        && hooks != &paths.mcp_config
        && let Some(existing) = read_optional(hooks)?
    {
        let desired = remove_hooks(&existing)?;
        actions.push(apply_content(
            hooks,
            Some(&existing),
            desired.as_bytes(),
            ActionKind::Hooks,
            options.dry_run,
        )?);
    }

    Ok(SetupResult {
        agent: adapter.slug,
        dry_run: options.dry_run,
        actions,
    })
}

pub(super) fn to_json_like(existing: &str, value: &serde_json::Value) -> Result<String> {
    let indent = detected_indent(existing);
    let mut output = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(
        &mut output,
        serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes()),
    );
    serde::Serialize::serialize(value, &mut serializer).context("serialize JSON configuration")?;
    Ok(with_line_endings_of(
        existing,
        format!("{}\n", String::from_utf8(output)?),
    ))
}

fn detected_indent(existing: &str) -> String {
    existing
        .lines()
        .find_map(|line| {
            let indent: String = line
                .chars()
                .take_while(|character| *character == ' ' || *character == '\t')
                .collect();
            (!indent.is_empty() && indent.len() < line.len()).then_some(indent)
        })
        .unwrap_or_else(|| "  ".to_owned())
}

fn remove_json_server(existing: &[u8], format: McpFormat) -> Result<String> {
    let mut config: Value =
        serde_json::from_slice(existing).context("MCP configuration is not valid JSON")?;
    // ZCode's hooks share this file — see `zcode.rs` — so taking the server
    // out takes them, exactly as `remove_codex_server` does for the TOML its
    // hooks live in. The generic hook-removal step in `uninstall` is skipped
    // for a file that holds both.
    if format == McpFormat::Zcode {
        remove_nested_leteo_hooks(&mut config);
    }
    if let Some(servers) = walk_to_servers_mut(&mut config, format) {
        servers.remove(SERVER_NAME);
    }
    to_json_like(std::str::from_utf8(existing).unwrap_or_default(), &config)
}

fn walk_to_servers_mut(
    config: &mut Value,
    format: McpFormat,
) -> Option<&mut serde_json::Map<String, Value>> {
    let path = format.key_path();
    let (last, parents) = path.split_last()?;
    let mut node = config;
    for key in parents {
        node = node.get_mut(key)?;
    }
    node.get_mut(last).and_then(Value::as_object_mut)
}

fn open_servers_mut<'a>(
    object: &'a mut serde_json::Map<String, Value>,
    format: McpFormat,
    path: &Path,
) -> Result<&'a mut serde_json::Map<String, Value>> {
    let keys = format.key_path();
    let mut node = object;
    for (index, key) in keys.iter().enumerate() {
        let entry = node
            .entry((*key).to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        if entry.is_null() {
            *entry = Value::Object(Map::new());
        }
        let last = index + 1 == keys.len();
        match entry.as_object_mut() {
            Some(inner) => {
                if last {
                    return Ok(inner);
                }
                node = inner;
            }
            None => anyhow::bail!(
                "{} in {} must contain a JSON object",
                keys[..=index].join("."),
                path.display()
            ),
        }
    }
    unreachable!("a non-empty key path always returns inside the loop")
}

fn remove_codex_server(existing: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(existing).context("Codex config is not valid UTF-8")?;
    let normalized = text.replace("\r\n", "\n");
    let lines = normalized.lines().collect::<Vec<_>>();
    let mut kept = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        if is_leteo_codex_section(lines[index]) {
            index += 1;
            while index < lines.len() && !is_toml_section(lines[index]) {
                index += 1;
            }
            continue;
        }
        kept.push(lines[index]);
        index += 1;
    }
    let kept = render::without_leteo_codex_hooks(&kept);
    let body = kept.join("\n");
    Ok(with_line_endings_of(text, format!("{}\n", body.trim_end())))
}

pub fn remove_memory_protocol(existing: &str) -> String {
    let text = existing.replace("\r\n", "\n");
    let stripped = strip_memory_protocol_blocks(&text);
    if stripped == text {
        return existing.to_owned();
    }
    let trimmed = stripped.trim();
    let desired = if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    };
    with_line_endings_of(existing, desired)
}

fn with_line_endings_of(source: &str, text: String) -> String {
    if source.contains("\r\n") {
        text.replace('\n', "\r\n")
    } else {
        text
    }
}

fn remove_hooks(existing: &[u8]) -> Result<String> {
    let mut settings: Value =
        serde_json::from_slice(existing).context("hook settings are not valid JSON")?;
    if let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) {
        for entries in hooks.values_mut() {
            if let Some(entries) = entries.as_array_mut() {
                entries.retain(|entry| !is_leteo_hook_entry(entry));
            }
        }
        hooks.retain(|_, entries| entries.as_array().is_none_or(|list| !list.is_empty()));
        let empty = hooks.is_empty();
        if empty && let Some(settings) = settings.as_object_mut() {
            settings.remove("hooks");
        }
    }
    to_json_like(std::str::from_utf8(existing).unwrap_or_default(), &settings)
}

fn zcode_hook_runner_disabled(config: &Value) -> bool {
    config.get("hooks").and_then(|hooks| hooks.get("enabled")) == Some(&Value::Bool(false))
}

fn remove_nested_leteo_hooks(config: &mut Value) {
    let Some(hooks) = config.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(events) = hooks.get_mut("events").and_then(Value::as_object_mut) else {
        return;
    };
    for entries in events.values_mut() {
        if let Some(entries) = entries.as_array_mut() {
            entries.retain(|entry| !is_leteo_hook_entry(entry));
        }
    }
    events.retain(|_, entries| entries.as_array().is_none_or(|list| !list.is_empty()));
    if events.is_empty() {
        hooks.remove("events");
    }
}

pub fn setup(agent: &str, options: &SetupOptions) -> Result<SetupResult> {
    let adapter = find_adapter(agent)?;
    let environment = SetupEnvironment::resolve(options)?;
    refuse_a_path_that_will_not_be_there(&environment.executable)?;
    let paths = resolve_paths(adapter, &environment);

    let existing_config = read_optional(&paths.mcp_config)?;

    // ZCode's hook runner starts switched off, and Leteo refuses rather than
    // write registrations the client will not read. The question is asked
    // here, before anything is written: its hooks share the MCP config file,
    // so a refusal that arrived after the server step would leave a
    // half-install — a new server entry reporting failure beside it.
    if options.install_hooks
        && adapter.config_format == ConfigFormat::Json(McpFormat::Zcode)
        && existing_config
            .as_deref()
            .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
            .is_some_and(|config| zcode_hook_runner_disabled(&config))
    {
        anyhow::bail!(
            "{} sets hooks.enabled to false, which tells this client to run no \
             configuration hooks at all — Leteo's registrations would be written \
             and then ignored. Turn it back on there and run this again.",
            paths.mcp_config.display()
        );
    }
    let desired_config = match adapter.config_format {
        ConfigFormat::Json(format) => render_json_config(
            &paths.mcp_config,
            existing_config.as_deref(),
            format,
            &environment.executable,
            options.tools.as_deref().unwrap_or(DEFAULT_TOOLS),
        )?,
        ConfigFormat::CodexToml => render_codex_config(
            existing_config.as_deref(),
            executable_string(&environment.executable)?,
            options.tools.as_deref().unwrap_or(DEFAULT_TOOLS),
        )?,
        ConfigFormat::DshPatch => render_dsh_patch_config(
            existing_config.as_deref(),
            executable_string(&environment.executable)?,
            options.tools.as_deref().unwrap_or(DEFAULT_TOOLS),
        )?,
    };

    let mut actions = vec![apply_content(
        &paths.mcp_config,
        existing_config.as_deref(),
        desired_config.as_bytes(),
        ActionKind::McpConfiguration,
        options.dry_run,
    )?];

    if options.install_instructions {
        let instructions_path = paths.instructions.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "{} has no instruction file; it receives the memory protocol at runtime",
                adapter.display_name
            )
        })?;
        let existing_instructions = read_optional(instructions_path)?;
        let existing_text = existing_instructions
            .as_deref()
            .map(|content| {
                std::str::from_utf8(content)
                    .with_context(|| format!("{} is not valid UTF-8", instructions_path.display()))
            })
            .transpose()?
            .unwrap_or_default();
        let desired_instructions =
            render_instructions(adapter, existing_text, existing_instructions.is_none());
        actions.push(apply_content(
            instructions_path,
            existing_instructions.as_deref(),
            desired_instructions.as_bytes(),
            ActionKind::Instructions,
            options.dry_run,
        )?);
    }

    if options.install_hooks {
        let hooks_path = paths.hooks.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "{} does not support Leteo lifecycle hooks yet",
                adapter.display_name
            )
        })?;
        if let Some(bundle) = installed_plugin_hooks(&environment, adapter) {
            anyhow::bail!(
                "the Leteo plugin already registers these hooks at {}, and installing them \
                 again would run every event twice — storing each prompt twice. Run without \
                 --hooks, or remove the plugin first.",
                bundle.display()
            );
        }
        // For Codex the hooks and the server share one file. Building on what
        // the server write just decided, rather than on what was read before
        // it, keeps the two from racing — and matters most under `--dry-run`,
        // where nothing is on disk to read back and the hooks would otherwise
        // be reported as dropping the server they were meant to sit beside.
        let existing_hooks = if hooks_path == &paths.mcp_config {
            Some(desired_config.into_bytes())
        } else {
            read_optional(hooks_path)?
        };
        let desired_hooks = if adapter.config_format == ConfigFormat::Json(McpFormat::Zcode) {
            render::render_zcode_hooks(
                hooks_path,
                existing_hooks.as_deref(),
                adapter.hook_registrations,
                &environment.executable,
            )?
        } else {
            match adapter.config_format {
                ConfigFormat::CodexToml => render::render_codex_hooks(
                    existing_hooks.as_deref(),
                    adapter.hook_registrations,
                    &environment.executable,
                )?,
                ConfigFormat::Json(_) => render::render_hooks_config(
                    hooks_path,
                    existing_hooks.as_deref(),
                    adapter.hook_registrations,
                    &environment.executable,
                )?,
                ConfigFormat::DshPatch => unreachable!("DeepSeek Harness takes no hooks"),
            }
        };
        actions.push(apply_content(
            hooks_path,
            existing_hooks.as_deref(),
            desired_hooks.as_bytes(),
            ActionKind::Hooks,
            options.dry_run,
        )?);
    }

    Ok(SetupResult {
        agent: adapter.slug,
        dry_run: options.dry_run,
        actions,
    })
}

/// Refuse to write a path that the package manager owns.
///
/// `setup` writes the absolute path of the running binary into an agent's
/// configuration, which is right for a binary somebody installed and wrong for
/// one npm is holding on their behalf. Run through the npm wrapper, that path
/// is inside the package's own cache:
///
/// ```text
/// "command": "/home/u/.npm/_npx/1a19a25e/node_modules/@asanabrial/leteo/vendor/…"
/// ```
///
/// and it goes away on `npm cache clean`, when npx evicts the entry, or when
/// the version changes. When it does, the MCP server stops starting and all
/// five hooks fail — silently, because a hook that fails says nothing by
/// design so that a broken Leteo never blocks a prompt. The memory simply
/// stops being recorded and nothing anywhere says why.
///
/// So it is refused where somebody is standing there to read the sentence,
/// rather than left to break later with nobody watching. `node_modules` is the
/// marker because it covers both routes npm has — the `_npx` cache and a global
/// install — and appears in neither of the places a person installs a binary.
fn refuse_a_path_that_will_not_be_there(executable: &Path) -> Result<()> {
    let managed = executable
        .components()
        .any(|component| component.as_os_str() == "node_modules");
    if managed {
        anyhow::bail!(
            "this Leteo is the one npm is holding, at {}, and that path is deleted by \
             `npm cache clean` or by the next version — the hooks written against it would \
             then fail silently. Point your client at the wrapper instead, which stays \
             put:\n\n  \"command\": \"npx\", \"args\": [\"-y\", \"@asanabrial/leteo\", \"mcp\"]\n\n\
             Or install Leteo itself and run this again: https://github.com/asanabrial/leteo#install",
            executable.display()
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookRegistration {
    pub event: &'static str,
    pub slug: &'static str,
    pub matcher: Option<&'static str>,
    pub timeout_seconds: u64,
}

/// The lifecycle events Leteo registers, with the matcher and timeout each one
/// needs. `session-stop` sits on `SessionEnd`, not on `Stop`.
///
/// `Stop` fires when the agent finishes a reply — at the end of every turn, not
/// at the end of the conversation. Registered there, the hook ended the session
/// on every turn and deleted the reminder debounce with it, which is what made
/// the save reminder appear on every single prompt instead of every fifteen
/// minutes. Anything named for a session belongs on the event that means the
/// session is over.
pub const SESSION_START: HookRegistration = HookRegistration {
    event: "SessionStart",
    slug: "session-start",
    matcher: Some("startup|clear"),
    timeout_seconds: 10,
};
pub const POST_COMPACTION: HookRegistration = HookRegistration {
    event: "SessionStart",
    slug: "post-compaction",
    matcher: Some("compact"),
    timeout_seconds: 10,
};
pub const USER_PROMPT_SUBMIT: HookRegistration = HookRegistration {
    event: "UserPromptSubmit",
    slug: "user-prompt-submit",
    matcher: None,
    timeout_seconds: 5,
};
pub const SUBAGENT_STOP: HookRegistration = HookRegistration {
    event: "SubagentStop",
    slug: "subagent-stop",
    matcher: None,
    timeout_seconds: 10,
};
// Three, not five, because Codex caps this one: asking for more earns
// "clamping SessionEnd hook timeout to 3s" on every single session. It is
// the event that runs while the agent is shutting down, so the cap is fair.
// One number rather than one per agent, because there is room — against a
// 3530-memory store `session-stop` takes 34ms, and 742ms on a cold start.
pub const SESSION_END: HookRegistration = HookRegistration {
    event: "SessionEnd",
    slug: "session-stop",
    matcher: None,
    timeout_seconds: 3,
};

const HOOK_EVENTS: &[HookRegistration] = &[
    SESSION_START,
    POST_COMPACTION,
    USER_PROMPT_SUBMIT,
    SUBAGENT_STOP,
    SESSION_END,
];

/// What ZCode can actually fire: three of the five.
///
/// Its client supports exactly seven events (`SessionStart`, `UserPromptSubmit`,
/// `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PostToolUseFailure`,
/// `Stop`) — there is no `SubagentStop` and no `SessionEnd`, so those two
/// hooks have nowhere to land. `session-stop` is *not* moved onto `Stop` for
/// the reason written on `SESSION_END`: `Stop` fires at the end of every
/// reply, where ending a session deletes the reminder debounce and broke the
/// save reminder for real once. On ZCode the closing summary therefore comes
/// from the agent calling `mem_session_summary` itself, or not at all.
pub const ZCODE_HOOK_REGISTRATIONS: &[HookRegistration] =
    &[SESSION_START, POST_COMPACTION, USER_PROMPT_SUBMIT];

/// Whether a command line runs one of Leteo's own hooks.
///
/// # Why the subcommand is not enough
///
/// This asked only whether the text held `hook session-start`, and that is not
/// a name Leteo owns. `session-start`, `session-end` and `user-prompt-submit`
/// are the events *the agent* defines, so any tool hooking the same agent
/// spells its commands the same way — the machine this was found on had
/// `warden.exe hook session-start` sitting in the same `settings.json`.
///
/// Every caller either deletes what this matches or reports it as Leteo's, so
/// a false positive was a tool quietly eating another tool's configuration on
/// install, on uninstall, and a `doctor` that told the person to remove hooks
/// Leteo had never written.
///
/// So the executable is checked too. It is the last path segment before the
/// subcommand, with the surrounding quotes and any `.exe` taken off, and it has
/// to be `leteo`. Being wrong in this direction costs a hook of ours left
/// behind for somebody who renamed the binary; being wrong in the other costs
/// somebody else's work.
pub(super) fn runs_a_leteo_hook(command: &str) -> bool {
    HOOK_EVENTS.iter().any(|registration| {
        command
            .match_indices(&format!("hook {}", registration.slug))
            .any(|(at, _)| names_the_leteo_binary(&command[..at]))
    })
}

/// Whether what precedes a `hook <slug>` is Leteo's own binary.
///
/// Works on raw file text as well as on a parsed JSON string, which is why the
/// escaping is stripped rather than parsed: in `settings.json` on disk the same
/// command reads `\"C:\\...\\leteo.exe\" hook session-start`, and both forms
/// have to answer the same.
fn names_the_leteo_binary(before: &str) -> bool {
    let executable = before.trim_end().trim_end_matches(['"', '\'', '\\']);
    let name = executable
        .rsplit(['/', '\\', ' ', '\t', '"', '\''])
        .next()
        .unwrap_or(executable);
    let name = name.strip_suffix(".exe").unwrap_or(name);
    name.eq_ignore_ascii_case("leteo")
}

fn is_leteo_hook_entry(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(runs_a_leteo_hook)
            })
        })
}

impl SetupEnvironment {
    fn resolve(options: &SetupOptions) -> Result<Self> {
        let platform = options.platform.unwrap_or_else(Platform::current);
        let home = match &options.home_dir {
            Some(path) => path.clone(),
            None => system_home_dir()?,
        };
        require_absolute(&home, "home directory")?;

        let config_home = resolve_optional_root(
            options.config_home.clone(),
            env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        );
        let app_data = resolve_optional_root(
            options.app_data.clone(),
            env::var_os("APPDATA").map(PathBuf::from),
        );
        let executable = match &options.executable {
            Some(path) => path.clone(),
            None => env::current_exe().context("resolve the Leteo executable")?,
        };
        require_absolute(&executable, "Leteo executable")?;
        // Canonicalization resolves symlinks, but on Windows it also returns a
        // `\\?\` verbatim path that agent launchers refuse to execute.
        let executable = crate::project::remove_windows_verbatim_prefix(
            executable.canonicalize().unwrap_or(executable),
        );

        Ok(Self {
            platform,
            home,
            config_home,
            app_data,
            executable,
            // Read once here rather than at each place a Claude path is built,
            // so one run cannot resolve half its paths against one directory
            // and half against another.
            claude_config: resolve_optional_root(
                None,
                env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
            ),
            dsh_home: resolve_optional_root(
                options.dsh_home.clone(),
                env::var_os("DSH_HOME").map(PathBuf::from),
            ),
        })
    }

    fn xdg_config_root(&self) -> PathBuf {
        self.config_home
            .clone()
            .unwrap_or_else(|| self.home.join(".config"))
    }

    fn roaming_root(&self) -> PathBuf {
        self.app_data
            .clone()
            .unwrap_or_else(|| self.home.join("AppData").join("Roaming"))
    }

    fn pi_agent_dir(&self) -> PathBuf {
        env::var_os("PI_CODING_AGENT_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| self.home.join(".pi").join("agent"))
    }

    fn dsh_home_dir(&self) -> PathBuf {
        self.dsh_home
            .clone()
            .unwrap_or_else(|| self.home.join(".dsh"))
    }
}

fn find_adapter(agent: &str) -> Result<&'static AgentAdapter> {
    agents::REGISTRY
        .iter()
        .find(|adapter| adapter.slug == agent)
        .ok_or_else(|| {
            let supported = agents::REGISTRY
                .iter()
                .map(|adapter| adapter.slug)
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::anyhow!("unknown agent {agent:?}; supported agents: {supported}")
        })
}

fn resolve_optional_root(
    explicit: Option<PathBuf>,
    environment: Option<PathBuf>,
) -> Option<PathBuf> {
    explicit.or(environment).filter(|path| path.is_absolute())
}

fn system_home_dir() -> Result<PathBuf> {
    crate::paths::home_dir()
}

fn require_absolute(path: &Path, name: &str) -> Result<()> {
    if !path.is_absolute() {
        bail!("{name} must be absolute, got {}", path.display());
    }
    Ok(())
}

fn executable_string(executable: &Path) -> Result<&str> {
    executable.to_str().with_context(|| {
        format!(
            "Leteo executable path is not valid UTF-8: {}",
            executable.display()
        )
    })
}

fn mcp_entry(format: McpFormat, executable: &Path, tools: &str) -> Result<Value> {
    let command = executable_string(executable)?;
    let profile = format!("--tools={tools}");
    Ok(match format {
        McpFormat::McpServers => json!({
            "command": command,
            "args": ["mcp", profile]
        }),
        McpFormat::Servers => json!({
            "type": "stdio",
            "command": command,
            "args": ["mcp", profile]
        }),
        McpFormat::Mcp => json!({
            "type": "local",
            "command": [command, "mcp", profile],
            "enabled": true
        }),
        // `eager` starts the server when the session does. Pi has no MCP of its
        // own — `pi-mcp-extension` reads this file — and in that extension
        // `lazy` does not mean "start on first use": it means the server stays
        // down until somebody types `/mcp:start leteo`, and it is the default
        // that applies when the field is left out. Leteo was writing it, so its
        // tools were absent from every Pi session that did not turn them on by
        // hand. A memory you have to remember to switch on is the one thing
        // this cannot be.
        McpFormat::Pi => json!({
            "command": command,
            "args": ["mcp", profile],
            "lifecycle": "eager"
        }),
        McpFormat::Zcode => json!({
            "type": "stdio",
            "command": command,
            "args": ["mcp", profile]
        }),
    })
}

pub const DEFAULT_TOOLS: &str = "agent";

fn render_codex_config(existing: Option<&[u8]>, executable: &str, tools: &str) -> Result<String> {
    let existing = match existing {
        Some(content) => std::str::from_utf8(content).context("Codex config is not valid UTF-8")?,
        None => "",
    };
    let normalized = existing.replace("\r\n", "\n");
    let lines = normalized.lines().collect::<Vec<_>>();
    let mut kept = Vec::with_capacity(lines.len());
    let mut index = 0;

    while index < lines.len() {
        if is_leteo_codex_section(lines[index]) {
            index += 1;
            while index < lines.len() && !is_toml_section(lines[index]) {
                index += 1;
            }
            continue;
        }
        kept.push(lines[index]);
        index += 1;
    }

    let command = serde_json::to_string(executable).context("quote Leteo executable for TOML")?;
    let block = format!(
        "[mcp_servers.{SERVER_NAME}]\ncommand = {command}\nargs = [\"mcp\", \"--tools={tools}\"]"
    );
    let base = kept.join("\n");
    let base = base.trim_end();
    let desired = if base.is_empty() {
        format!("{block}\n")
    } else {
        format!("{base}\n\n{block}\n")
    };
    Ok(with_line_endings_of(existing, desired))
}

fn is_leteo_codex_section(line: &str) -> bool {
    let line = line.trim();
    line == "[mcp_servers.leteo]"
        || line == "[mcp_servers.\"leteo\"]"
        || line.starts_with("[mcp_servers.leteo.")
        || line.starts_with("[mcp_servers.\"leteo\".")
}

fn is_toml_section(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('[') && line.ends_with(']')
}

pub fn upsert_memory_protocol(existing: &str) -> String {
    let text = existing.replace("\r\n", "\n");
    let block = format!(
        "{MEMORY_PROTOCOL_BEGIN}\n\n{}\n\n{MEMORY_PROTOCOL_END}",
        MEMORY_PROTOCOL.trim()
    );

    let Some(start) = text.find(MEMORY_PROTOCOL_BEGIN) else {
        let trimmed = text.trim_end();
        let desired = if trimmed.is_empty() {
            format!("{block}\n")
        } else {
            format!("{trimmed}\n\n{block}\n")
        };
        return with_line_endings_of(existing, desired);
    };

    // A block that opens and never closes: a hand-edit that took the end marker
    // with it, or a write that did not finish. Where it was meant to stop cannot
    // be known, and guessing costs whatever the person wrote after it — so the
    // block is closed where it opened and the orphaned body stays in the file as
    // ordinary text, visible and theirs to delete.
    //
    // Appending a second block instead, which is what this did, left the file
    // with two begin markers. The next run spliced from the first marker to the
    // second block's end marker and took everything in between with it.
    let end = match text[start..].find(MEMORY_PROTOCOL_END) {
        Some(relative_end) => start + relative_end + MEMORY_PROTOCOL_END.len(),
        None => start + MEMORY_PROTOCOL_BEGIN.len(),
    };
    let desired = format!(
        "{}{block}{}",
        &text[..start],
        strip_memory_protocol_blocks(&text[end..])
    );
    with_line_endings_of(existing, desired)
}

fn strip_memory_protocol_blocks(text: &str) -> String {
    let mut rest = text;
    let mut kept = String::new();
    while let Some(start) = rest.find(MEMORY_PROTOCOL_BEGIN)
        && let Some(relative_end) = rest[start..].find(MEMORY_PROTOCOL_END)
    {
        kept.push_str(rest[..start].trim_end());
        rest = &rest[start + relative_end + MEMORY_PROTOCOL_END.len()..];
    }
    kept.push_str(rest);
    kept
}

fn render_instructions(adapter: &AgentAdapter, existing: &str, is_new: bool) -> String {
    let existing = if is_new {
        adapter.new_instruction_file
    } else {
        existing
    };
    upsert_memory_protocol(existing)
}

fn strip_jsonc(content: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(content.len());
    let mut index = 0;

    while index < content.len() {
        if content[index] == b'"' {
            output.push(content[index]);
            index += 1;
            while index < content.len() {
                output.push(content[index]);
                if content[index] == b'\\' && index + 1 < content.len() {
                    index += 1;
                    output.push(content[index]);
                } else if content[index] == b'"' {
                    index += 1;
                    break;
                }
                index += 1;
            }
            continue;
        }
        if content.get(index..index + 2) == Some(b"//") {
            index += 2;
            while index < content.len() && content[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if content.get(index..index + 2) == Some(b"/*") {
            index += 2;
            while index + 1 < content.len() && &content[index..index + 2] != b"*/" {
                index += 1;
            }
            index = (index + 2).min(content.len());
            continue;
        }
        output.push(content[index]);
        index += 1;
    }

    strip_trailing_commas(&output)
}

fn strip_trailing_commas(content: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(content.len());
    let mut index = 0;
    let mut in_string = false;

    while index < content.len() {
        let byte = content[index];
        if byte == b'"' {
            output.push(byte);
            index += 1;
            in_string = !in_string;
            continue;
        }
        if in_string && byte == b'\\' && index + 1 < content.len() {
            output.push(byte);
            index += 1;
            output.push(content[index]);
            index += 1;
            continue;
        }
        if !in_string && byte == b',' {
            let mut next = index + 1;
            while next < content.len() && content[next].is_ascii_whitespace() {
                next += 1;
            }
            if content
                .get(next)
                .is_some_and(|byte| matches!(byte, b'}' | b']'))
            {
                index += 1;
                continue;
            }
        }
        output.push(byte);
        index += 1;
    }

    output
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

fn still_configured_sharer(
    adapter: &AgentAdapter,
    instructions: &Path,
    environment: &SetupEnvironment,
) -> Result<Option<&'static str>> {
    for other in agents::REGISTRY {
        if other.slug == adapter.slug {
            continue;
        }
        let paths = resolve_paths(other, environment);
        if paths.instructions.as_deref() != Some(instructions) {
            continue;
        }
        let Some(config) = read_optional(&paths.mcp_config)? else {
            continue;
        };
        if names_leteo(&config, other.config_format) {
            return Ok(Some(other.slug));
        }
    }
    Ok(None)
}

fn names_leteo(config: &[u8], format: ConfigFormat) -> bool {
    let Ok(text) = std::str::from_utf8(config) else {
        return true;
    };
    match format {
        ConfigFormat::Json(format) => match serde_json::from_str::<Value>(text) {
            Ok(value) => {
                servers_at(&value, format).is_some_and(|servers| servers.contains_key(SERVER_NAME))
            }
            Err(_) => true,
        },
        ConfigFormat::CodexToml => text.contains(&format!("mcp_servers.{SERVER_NAME}")),
        ConfigFormat::DshPatch => dsh_names_leteo(text),
    }
}

fn remove_file(path: &Path, kind: ActionKind, dry_run: bool) -> Result<SetupAction> {
    if !dry_run {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("remove {}", path.display())),
        }
    }
    Ok(SetupAction {
        kind,
        path: path.to_owned(),
        change: Change::Removed,
        kept_for: None,
    })
}

fn apply_content(
    path: &Path,
    existing: Option<&[u8]>,
    desired: &[u8],
    kind: ActionKind,
    dry_run: bool,
) -> Result<SetupAction> {
    let change = match existing {
        None => Change::Create,
        Some(content) if content == desired => Change::Unchanged,
        Some(_) => Change::Update,
    };

    if !dry_run && change != Change::Unchanged {
        // Every file this touches belongs to somebody else, and most of them
        // hold text that was written by hand. A truncating write that does not
        // finish leaves one that is neither the old file nor the new one.
        crate::files::replace(path, desired)
            .with_context(|| format!("write {}", path.display()))?;
    }

    Ok(SetupAction {
        kind,
        path: path.to_owned(),
        change,
        kept_for: None,
    })
}

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Serialize)]
pub struct HookHealth {
    pub agent: &'static str,
    pub display_name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundled: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
}

pub fn hook_health(options: &SetupOptions) -> Vec<HookHealth> {
    let Ok(environment) = SetupEnvironment::resolve(options) else {
        return Vec::new();
    };
    supported_agents()
        .iter()
        .filter(|adapter| adapter.supports_hooks())
        .map(|adapter| {
            let configured = resolve_paths(adapter, &environment)
                .hooks
                .filter(|path| file_registers_leteo_hooks(path));
            let bundled = installed_plugin_hooks(&environment, adapter);
            let issue = if configured.is_some() && bundled.is_some() {
                Some(format!(
                    "{} runs every Leteo hook twice: they are registered both by \
                     the plugin bundle and by `leteo setup {} --hooks`. Remove \
                     the plugin, or run `leteo setup {} --uninstall`.",
                    adapter.display_name, adapter.slug, adapter.slug
                ))
            } else if (configured.is_some() || bundled.is_some())
                && matches!(adapter.config_format, ConfigFormat::CodexToml)
                && !codex_trusts_any_hook(&environment)
            {
                Some(format!(
                    "{} has hooks installed but has not been told to trust any \
                     of them, so none of them run. Start a session and choose \
                     \"Trust all and continue\" on the hooks review screen.",
                    adapter.display_name
                ))
            } else {
                hook_runner_switch_is_off(&environment, adapter)
                    .filter(|_| configured.is_some() || bundled.is_some())
                    .map(|switch| {
                        format!(
                            "{} has hooks installed but {} sets hooks.enabled to \
                             false, so none of them run. Set it back to true there.",
                            adapter.display_name,
                            switch.display()
                        )
                    })
            };
            HookHealth {
                agent: adapter.slug,
                display_name: adapter.display_name,
                configured,
                bundled,
                issue,
            }
        })
        .collect()
}

fn file_registers_leteo_hooks(path: &Path) -> bool {
    fs::read_to_string(path).is_ok_and(|text| runs_a_leteo_hook(&text))
}

fn codex_trusts_any_hook(environment: &SetupEnvironment) -> bool {
    fs::read_to_string(environment.home.join(".codex").join("config.toml"))
        .is_ok_and(|text| text.contains("trusted_hash"))
}

pub fn plugin_registers_hooks(agent: &str, options: &SetupOptions) -> bool {
    let Ok(adapter) = find_adapter(agent) else {
        return false;
    };
    SetupEnvironment::resolve(options)
        .ok()
        .and_then(|environment| installed_plugin_hooks(&environment, adapter))
        .is_some()
}

fn hook_runner_switch_is_off(
    environment: &SetupEnvironment,
    adapter: &AgentAdapter,
) -> Option<PathBuf> {
    if adapter.config_format != ConfigFormat::Json(McpFormat::Zcode) {
        return None;
    }
    let path = resolve_paths(adapter, environment).hooks?;
    let existing = read_optional(&path).ok().flatten()?;
    let config = serde_json::from_slice::<Value>(&existing).ok()?;
    zcode_hook_runner_disabled(&config).then_some(path)
}

pub fn hook_runner_switched_off(agent: &str, options: &SetupOptions) -> bool {
    let Ok(adapter) = find_adapter(agent) else {
        return false;
    };
    SetupEnvironment::resolve(options)
        .ok()
        .and_then(|environment| hook_runner_switch_is_off(&environment, adapter))
        .is_some()
}

fn installed_plugin_hooks(
    environment: &SetupEnvironment,
    adapter: &AgentAdapter,
) -> Option<PathBuf> {
    let cache = adapter.plugin_cache_root?(environment)
        .join("plugins")
        .join("cache");
    let mut directories = vec![cache];
    for _ in 0..3 {
        let mut next = Vec::new();
        for directory in directories {
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            next.extend(
                entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir()),
            );
        }
        directories = next;
    }
    directories.into_iter().find_map(|directory| {
        let manifest = directory.join("hooks").join("hooks.json");
        let text = fs::read_to_string(&manifest).ok()?;
        text.contains("leteo hook").then_some(manifest)
    })
}
