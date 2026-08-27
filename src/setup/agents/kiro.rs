use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "kiro",
    display_name: "Kiro",
    config_format: ConfigFormat::Json(McpFormat::McpServers),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: true,
    hooks_path: None,
    hook_registrations: super::ALL_HOOK_REGISTRATIONS,
    plugin_cache_root: None,
};

fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment
        .home
        .join(".kiro")
        .join("settings")
        .join("mcp.json")
}

/// Kiro reads guidance from `steering/`, a sibling of `settings/` rather than
/// a file beside the configuration.
fn instructions(environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    environment
        .home
        .join(".kiro")
        .join("steering")
        .join("leteo.md")
}
