//! Cursor.

use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "cursor",
    display_name: "Cursor",
    config_format: ConfigFormat::Json(McpFormat::McpServers),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: true,
    hooks_path: None,
};

fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".cursor").join("mcp.json")
}

fn instructions(_environment: &SetupEnvironment, config: &Path) -> PathBuf {
    config.with_file_name("leteo-memory-protocol.md")
}
