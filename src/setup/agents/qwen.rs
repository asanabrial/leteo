//! Qwen.

use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "qwen",
    display_name: "Qwen",
    config_format: ConfigFormat::Json(McpFormat::McpServers),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: None,
};

fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".qwen").join("settings.json")
}

fn instructions(_environment: &SetupEnvironment, config: &Path) -> PathBuf {
    config.with_file_name("QWEN.md")
}
