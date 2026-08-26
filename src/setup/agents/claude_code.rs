//! Claude Code.

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

fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".claude.json")
}

fn instructions(environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    environment.home.join(".claude").join("CLAUDE.md")
}

/// The only agent with a stable, documented hook settings file today.
fn hooks(environment: &SetupEnvironment) -> PathBuf {
    environment.claude_config_dir().join("settings.json")
}

/// Its plugins cache under the config directory the way its settings do,
/// which is why `CLAUDE_CONFIG_DIR` moves both.
fn plugin_cache(environment: &SetupEnvironment) -> PathBuf {
    environment.claude_config_dir()
}
