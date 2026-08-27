use std::path::PathBuf;

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

/// No instruction file: Pi's packages inject guidance at runtime, so writing
/// one would leave a file Pi never reads.
pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "pi",
    display_name: "Pi",
    config_format: ConfigFormat::Json(McpFormat::Pi),
    config_path,
    instruction_path: None,
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: None,
    hook_registrations: super::ALL_HOOK_REGISTRATIONS,
    plugin_cache_root: None,
};

fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment.pi_agent_dir().join("mcp.json")
}
