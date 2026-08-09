//! Windsurf.

use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "windsurf",
    display_name: "Windsurf",
    config_format: ConfigFormat::Json(McpFormat::McpServers),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: None,
};

fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment
        .home
        .join(".codeium")
        .join("windsurf")
        .join("mcp_config.json")
}

fn instructions(_environment: &SetupEnvironment, config: &Path) -> PathBuf {
    config
        .parent()
        .expect("Windsurf MCP config has a parent")
        .join("memories")
        .join("global_rules.md")
}
