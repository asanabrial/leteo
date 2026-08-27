use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "kilocode",
    display_name: "Kilo Code",
    config_format: ConfigFormat::Json(McpFormat::Mcp),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: None,
    hook_registrations: super::ALL_HOOK_REGISTRATIONS,
    plugin_cache_root: None,
};

fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment
        .xdg_config_root()
        .join("kilo")
        .join("opencode.json")
}

fn instructions(_environment: &SetupEnvironment, config: &Path) -> PathBuf {
    config.with_file_name("AGENTS.md")
}
