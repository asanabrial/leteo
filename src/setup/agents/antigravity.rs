//! Antigravity.

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
};

/// Antigravity reads the shared Gemini MCP config, which is a different file
/// from the Gemini CLI's own `settings.json`.
fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment
        .home
        .join(".gemini")
        .join("config")
        .join("mcp_config.json")
}

fn instructions(environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    environment.home.join(".gemini").join("GEMINI.md")
}
