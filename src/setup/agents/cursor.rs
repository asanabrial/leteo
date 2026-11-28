use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "cursor",
    display_name: "Cursor",
    config_format: ConfigFormat::Json(McpFormat::McpServers),
    config_path,
    instruction_path: Some(instructions),
    // Cursor applies a project rule only when its frontmatter says so; with
    // `alwaysApply` absent the rule is used only when somebody @-mentions it,
    // and a plain `.md` in `rules/` is ignored outright.
    new_instruction_file: "---\nalwaysApply: true\n---\n",
    owns_instruction_file: true,
    hooks_path: None,
    hook_registrations: super::ALL_HOOK_REGISTRATIONS,
    plugin_cache_root: None,
};

fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".cursor").join("mcp.json")
}

/// The file Cursor loads as a user rule.
///
/// It was written to `~/.cursor/leteo-memory-protocol.md`, beside `mcp.json`,
/// where nothing reads it. User rule files live in `~/.cursor/rules` (Windows
/// `%USERPROFILE%\.cursor\rules`), and the rules system only accepts a `.mdc`
/// with frontmatter: a plain `.md` in that directory is ignored for having
/// none. Read at cursor.com/help/customization/rules and cursor.com/docs/rules
/// on 2026-11-28.
fn instructions(environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    environment
        .home
        .join(".cursor")
        .join("rules")
        .join("leteo-memory-protocol.mdc")
}
