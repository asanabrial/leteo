use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "zcode",
    display_name: "ZCode",
    config_format: ConfigFormat::Json(McpFormat::Zcode),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: Some(config_path),
    hook_registrations: super::ZCODE_HOOK_REGISTRATIONS,
    plugin_cache_root: Some(plugin_cache),
};

/// The user-scope configuration file. Verified in the client's own source:
/// `.zcode/cli` + `config.json`, with the servers under `mcp.servers` and
/// everything else it holds — providers, plugin state, its own hooks — read
/// back from the same document. ZCode's hooks land here too, one registration
/// per event under `hooks.events.<Event>`; keeping both kinds apart in one
/// document is why every edit splices in place rather than rewriting it.
///
/// There is no environment variable to move this directory the way
/// `CLAUDE_CONFIG_DIR` moves Claude's, so `home/.zcode` is not a guess.
fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment
        .home
        .join(".zcode")
        .join("cli")
        .join("config.json")
}

fn instructions(_environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    _environment.home.join(".zcode").join("AGENTS.md")
}

fn plugin_cache(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".zcode").join("cli")
}
