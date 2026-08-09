//! VS Code Copilot.

use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};
use crate::setup::Platform;

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "vscode-copilot",
    display_name: "VS Code Copilot",
    config_format: ConfigFormat::Json(McpFormat::Servers),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: FRONT_MATTER,
    owns_instruction_file: true,
    hooks_path: None,
};

/// Copilot applies an instruction file only when it declares what it applies
/// to. Written without this, the file is created successfully and then ignored,
/// which is the failure that looks most like success.
const FRONT_MATTER: &str = "---\napplyTo: \"**\"\n---\n";

fn config_path(environment: &SetupEnvironment) -> PathBuf {
    user_dir(environment).join("mcp.json")
}

fn instructions(_environment: &SetupEnvironment, config: &Path) -> PathBuf {
    config
        .parent()
        .expect("VS Code MCP config has a parent")
        .join("prompts")
        .join("leteo.instructions.md")
}

/// VS Code's per-user settings directory, which is somewhere different on all
/// three platforms.
fn user_dir(environment: &SetupEnvironment) -> PathBuf {
    match environment.platform {
        Platform::Windows => environment.roaming_root().join("Code").join("User"),
        Platform::MacOs => environment
            .home
            .join("Library")
            .join("Application Support")
            .join("Code")
            .join("User"),
        Platform::Unix => environment.xdg_config_root().join("Code").join("User"),
    }
}
