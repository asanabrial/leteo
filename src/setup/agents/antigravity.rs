use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "antigravity",
    display_name: "Antigravity",
    config_format: ConfigFormat::Json(McpFormat::McpServers),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: None,
    hook_registrations: super::ALL_HOOK_REGISTRATIONS,
    plugin_cache_root: None,
};

/// Antigravity's global MCP config.
///
/// `~/.gemini/config/mcp_config.json` is the current global file shared by the
/// App, the IDE and the CLI; `~/.gemini/antigravity/mcp_config.json` is the
/// legacy path it replaced. Read at antigravity.google/docs/cli/gcli-migration
/// on 2026-11-28, whose directory mapping names the global file and
/// `.agents/mcp_config.json` for a workspace.
fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment
        .home
        .join(".gemini")
        .join("config")
        .join("mcp_config.json")
}

/// The global developer context Antigravity consults in every workspace.
///
/// Read at antigravity.google/docs/cli/gcli-migration on 2026-11-28: global
/// constraints live in `~/.gemini/GEMINI.md`, and a workspace's own
/// `AGENTS.md`/`GEMINI.md` are parsed beside it.
fn instructions(environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    environment.home.join(".gemini").join("GEMINI.md")
}
