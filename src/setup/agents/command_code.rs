use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "command-code",
    display_name: "Command Code",
    config_format: ConfigFormat::Json(McpFormat::CommandCode),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: None,
    hook_registrations: super::ALL_HOOK_REGISTRATIONS,
    plugin_cache_root: None,
};

fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".commandcode").join("mcp.json")
}

fn instructions(environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    environment.home.join(".commandcode").join("AGENTS.md")
}
