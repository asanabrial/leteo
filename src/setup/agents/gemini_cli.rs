//! Gemini CLI.

use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, McpFormat, SetupEnvironment};

pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "gemini-cli",
    display_name: "Gemini CLI",
    config_format: ConfigFormat::Json(McpFormat::McpServers),
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: None,
    hook_registrations: super::ALL_HOOK_REGISTRATIONS,
    plugin_cache_root: None,
};

/// The Gemini CLI reads `~/.gemini/settings.json` on every platform. Its own
/// `settings.js` builds the path as `join(homedir(), '.gemini')` with no branch
/// on the operating system, so the `%APPDATA%\gemini\settings.json` this used to
/// write on Windows was a file nothing ever opened: setup reported success and
/// the agent started with no memory tools at all. It was found by asking a real
/// Gemini session to list its tools — it named the two servers configured in
/// `~/.gemini` and neither of the two configured in `%APPDATA%`.
fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment.home.join(".gemini").join("settings.json")
}

/// `GEMINI.md` is the context file the CLI loads as user memory, the same role
/// `CLAUDE.md` plays for Claude Code. The `system.md` written here before is
/// read only when `GEMINI_SYSTEM_MD` is set to something other than 0 or false,
/// and when it is read it *replaces the whole system prompt* — so the old path
/// was either ignored or, the day someone set that variable, would have thrown
/// away the agent's own instructions and left it with nothing but ours.
fn instructions(_environment: &SetupEnvironment, config: &Path) -> PathBuf {
    config.with_file_name("GEMINI.md")
}
