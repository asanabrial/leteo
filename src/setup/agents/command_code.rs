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

/// Command Code's user-scope MCP file.
///
/// Read at commandcode.ai/docs/mcp on 2026-11-28: the three scopes are
/// `~/.commandcode/projects/<slug>/mcp.json` (local, the default), `.mcp.json`
/// in a project root, and `~/.commandcode/mcp.json` (user), which
/// `cmd mcp add --scope user` writes. The entry this adapter writes —
/// `transport: "stdio"` and `enabled` beside `command`/`args` — is that page's
/// stdio entry shape.
fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".commandcode").join("mcp.json")
}

/// The user memory tier, loaded in every project.
///
/// Read at commandcode.ai/docs/memory on 2026-11-28: the three tiers are
/// `~/.commandcode/AGENTS.md`, `<project>/AGENTS.md` (else
/// `.commandcode/AGENTS.md`), and a subdirectory's own, and all that exist are
/// loaded additively.
fn instructions(environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    environment.home.join(".commandcode").join("AGENTS.md")
}
