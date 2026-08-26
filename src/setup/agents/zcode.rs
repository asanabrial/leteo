//! ZCode.

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
    // Three of five: this client has no SubagentStop and no SessionEnd, and
    // `session-stop` deliberately does not move onto Stop, which fires at the
    // end of every reply rather than at the end of a session. The rest of the
    // story is written beside ZCODE_HOOK_REGISTRATIONS, where the choice was
    // made once instead of here.
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

/// The user-default instruction file, loaded into every workspace before any
/// repository-level `AGENTS.md`.
fn instructions(_environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    _environment.home.join(".zcode").join("AGENTS.md")
}

/// Plugin bundles cache under the same CLI directory as the configuration,
/// walked as `marketplace / plugin / version / hooks / hooks.json`.
///
/// A Leteo bundle for ZCode does not exist today; the question is answered so
/// that if one ever ships, its registrations are found rather than assumed
/// absent.
fn plugin_cache(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".zcode").join("cli")
}
