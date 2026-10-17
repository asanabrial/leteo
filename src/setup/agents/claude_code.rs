use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "claude-code",
    display_name: "Claude Code",
    config_format: ConfigFormat::Json(McpFormat::McpServers),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: Some(hooks),
    hook_registrations: super::ALL_HOOK_REGISTRATIONS,
    plugin_cache_root: Some(plugin_cache),
};

/// `CLAUDE_CONFIG_DIR` moves `.claude.json` into it, and Claude Code then reads
/// its user-scope servers there and nowhere else: on a machine with the profile
/// at `~/.claude-personal`, the working `leteo` entry was in that directory's
/// `.claude.json` and `~/.claude.json` had none. Unset, the file sits in the home
/// directory rather than inside `~/.claude`, so this is not `claude_config_dir`.
fn config_path(environment: &SetupEnvironment) -> PathBuf {
    match &environment.claude_config {
        Some(directory) => directory.join(".claude.json"),
        None => environment.home.join(".claude.json"),
    }
}

fn instructions(environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    environment.claude_config_dir().join("CLAUDE.md")
}

/// The only agent with a stable, documented hook settings file today.
fn hooks(environment: &SetupEnvironment) -> PathBuf {
    environment.claude_config_dir().join("settings.json")
}

fn plugin_cache(environment: &SetupEnvironment) -> PathBuf {
    environment.claude_config_dir()
}
