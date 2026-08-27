use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "codex",
    display_name: "Codex",
    config_format: ConfigFormat::CodexToml,
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    // The same file the MCP server goes in. Codex reads hooks from
    // `config.toml` itself rather than from a settings file of its own, so
    // there is no second path to point at — and the two writes land in order,
    // the hooks reading back what the server write just left.
    hooks_path: Some(config_path),
    hook_registrations: super::ALL_HOOK_REGISTRATIONS,
    plugin_cache_root: Some(plugin_cache),
};

/// `~/.codex` on every platform, Windows included.
///
/// Codex keeps its home there whatever the operating system, and the Windows
/// branch that used to send this to `%APPDATA%\codex` wrote a config file Codex
/// never reads: `leteo setup codex` reported success, the setup screen read the
/// same file back and said Codex was already configured, and Codex itself had
/// never heard of Leteo. A wrong path is worse than a missing one — it fails
/// while claiming to have worked.
fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".codex").join("config.toml")
}

fn instructions(_environment: &SetupEnvironment, config: &Path) -> PathBuf {
    config.with_file_name("AGENTS.md")
}

fn plugin_cache(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".codex")
}
