//! OpenCode.

use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "opencode",
    display_name: "OpenCode",
    config_format: ConfigFormat::Json(McpFormat::Mcp),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: None,
};

/// OpenCode accepts either extension and writes whichever it finds. An existing
/// `.jsonc` is therefore the file to edit; writing the `.json` beside it would
/// leave the one OpenCode actually reads untouched.
fn config_path(environment: &SetupEnvironment) -> PathBuf {
    let root = environment.xdg_config_root().join("opencode");
    let jsonc = root.join("opencode.jsonc");
    if jsonc.is_file() {
        jsonc
    } else {
        root.join("opencode.json")
    }
}

fn instructions(environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    environment
        .xdg_config_root()
        .join("opencode")
        .join("AGENTS.md")
}
