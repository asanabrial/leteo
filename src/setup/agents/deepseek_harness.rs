use std::path::{Path, PathBuf};

use super::{AgentAdapter, ConfigFormat, SetupEnvironment};

/// No lifecycle hooks: DeepSeek Harness has no auto-discovered hook settings
/// file. Its two hook packages are bridges you load explicitly and point at a
/// hook config of your own (a `.claude/settings.json` you would also need),
/// which is the one surface Leteo deliberately stays out of.
pub(super) const ADAPTER: AgentAdapter = AgentAdapter {
    slug: "deepseek-harness",
    display_name: "DeepSeek Harness",
    config_format: ConfigFormat::DshPatch,
    config_path,
    instruction_path: Some(instructions),
    new_instruction_file: "",
    owns_instruction_file: false,
    hooks_path: None,
    hook_registrations: super::ALL_HOOK_REGISTRATIONS,
    plugin_cache_root: None,
};

/// The machine-global patch layer every profile reads, hot-reloaded.
///
/// Lives under the harness home, `~/.dsh` by default and `$DSH_HOME` when that
/// moves it — resolved once in [`SetupEnvironment::dsh_home_dir`], so a test
/// home and the real one agree. `cordis.patch.yml` is the last-but-most local
/// layer in the composed stack: bundle patches, then the profile's own file,
/// then this one, then `--patch`. A row it inserts applies to every session in
/// every profile, the web GUI included. There is no per-project `cordis.yml`
/// for an installer to edit, so this is the file `leteo setup` writes and reads
/// back. The path and the row shape are taken from the harness's own reference
/// rather than inferred.
fn config_path(environment: &SetupEnvironment) -> PathBuf {
    environment.dsh_home_dir().join("cordis.patch.yml")
}

fn instructions(environment: &SetupEnvironment, _config: &Path) -> PathBuf {
    environment.dsh_home_dir().join("AGENTS.md")
}
