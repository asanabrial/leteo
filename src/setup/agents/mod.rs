use std::path::{Path, PathBuf};

use super::{ConfigFormat, HOOK_EVENTS, HookRegistration, McpFormat, SetupEnvironment};

mod antigravity;
mod claude_code;
mod codex;
mod cursor;
mod deepseek_harness;
mod gemini_cli;
mod kilocode;
mod kiro;
mod opencode;
mod pi;
mod qwen;
mod vscode_copilot;
mod windsurf;
mod zcode;

pub(super) const ALL_HOOK_REGISTRATIONS: &[HookRegistration] = HOOK_EVENTS;

pub(super) use super::ZCODE_HOOK_REGISTRATIONS;

/// Everything the rest of setup needs to know about one agent.
///
/// Not comparable, deliberately. Two of these fields are function pointers, and
/// comparing those compares addresses that the compiler is free to merge or
/// move between codegen units — an equality that is true or false for reasons
/// nothing to do with the agent. An adapter's identity is its [`slug`], and
/// that is what callers compare.
///
/// [`slug`]: AgentAdapter::slug
#[derive(Debug, Clone, Copy)]
pub struct AgentAdapter {
    pub slug: &'static str,
    pub display_name: &'static str,
    pub config_format: ConfigFormat,
    pub(super) config_path: fn(&SetupEnvironment) -> PathBuf,
    pub(super) instruction_path: Option<fn(&SetupEnvironment, &Path) -> PathBuf>,
    pub(super) new_instruction_file: &'static str,
    pub(super) owns_instruction_file: bool,
    pub(super) hooks_path: Option<fn(&SetupEnvironment) -> PathBuf>,
    pub(super) hook_registrations: &'static [HookRegistration],
    pub(super) plugin_cache_root: Option<fn(&SetupEnvironment) -> PathBuf>,
}

impl AgentAdapter {
    pub fn supports_hooks(&self) -> bool {
        self.hooks_path.is_some()
    }
}

pub const REGISTRY: &[AgentAdapter] = &[
    opencode::ADAPTER,
    claude_code::ADAPTER,
    zcode::ADAPTER,
    gemini_cli::ADAPTER,
    codex::ADAPTER,
    deepseek_harness::ADAPTER,
    cursor::ADAPTER,
    windsurf::ADAPTER,
    vscode_copilot::ADAPTER,
    kilocode::ADAPTER,
    qwen::ADAPTER,
    kiro::ADAPTER,
    antigravity::ADAPTER,
    pi::ADAPTER,
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::setup::Platform;

    /// A machine each agent can be asked about, on a named platform.
    ///
    /// The home is absolute for whatever system is *running* the test, not for
    /// `platform`. `Path::is_absolute` answers by host rules, so a Unix-shaped
    /// home would make every path below look relative when the suite runs on
    /// Windows — a failure about the fixture rather than about the agents.
    fn environment(platform: Platform) -> SetupEnvironment {
        let home = if cfg!(windows) {
            PathBuf::from(r"C:\Users\someone")
        } else {
            PathBuf::from("/home/someone")
        };
        SetupEnvironment {
            platform,
            executable: home.join("bin").join("leteo"),
            home,
            config_home: None,
            app_data: None,
            claude_config: None,
            dsh_home: None,
        }
    }

    #[test]
    fn no_two_agents_are_the_same_agent() {
        // `find_adapter` takes the first slug that matches, so a duplicate does
        // not fail — it makes the second copy unreachable, and `leteo setup
        // <slug>` quietly configures the wrong one.
        let mut seen = BTreeMap::new();
        for adapter in REGISTRY {
            assert!(!adapter.slug.is_empty(), "an agent with no slug");
            assert!(
                seen.insert(adapter.slug, adapter.display_name).is_none(),
                "two agents answer to {:?}",
                adapter.slug
            );
        }
        assert_eq!(seen.len(), REGISTRY.len());
    }

    #[test]
    fn the_registry_splits_three_ways_and_the_counts_are_taken_from_it() {
        let mut theirs = Vec::new();
        let mut ours = Vec::new();
        let mut none = Vec::new();
        for adapter in REGISTRY {
            match (
                adapter.instruction_path.is_some(),
                adapter.owns_instruction_file,
            ) {
                (true, false) => theirs.push(adapter.slug),
                (true, true) => ours.push(adapter.slug),
                (false, _) => none.push(adapter.slug),
            }
        }
        assert_eq!(
            theirs.len(),
            10,
            "keep a file that was already theirs: {theirs:?}"
        );
        assert_eq!(
            ours,
            ["cursor", "vscode-copilot", "kiro"],
            "get a file of Leteo's"
        );
        assert_eq!(none, ["pi"], "read no instruction file at all");
        assert_eq!(theirs.len() + ours.len() + none.len(), REGISTRY.len());

        let with_hooks: Vec<_> = REGISTRY
            .iter()
            .filter(|adapter| adapter.supports_hooks())
            .map(|adapter| adapter.slug)
            .collect();
        assert_eq!(
            with_hooks,
            ["claude-code", "zcode", "codex"],
            "hooks setup can install"
        );
    }

    #[test]
    fn no_two_agents_write_to_the_same_file() {
        // Sharing a path means configuring one agent silently rewrites
        // another's file. Antigravity reads a Gemini config, but a *different*
        // one from the Gemini CLI's: `mcp_config.json` against `settings.json`.
        //
        // Their instruction file is the exception, and it is the one place two
        // agents may meet. Both products load `~/.gemini/GEMINI.md` — the CLI as
        // its context file, Antigravity as its global memories, each verified in
        // its own source — so there is no separate file to give them. Leteo's
        // block is spliced in by marker, which makes a second install a rewrite
        // of the same block rather than a duplicate, and `uninstall` leaves the
        // block alone while the other agent still names Leteo. Only instructions
        // may be shared: two agents over one MCP config or one hooks file would
        // still be an overwrite, and stay an error.
        //
        // An agent naming the same file twice is a different matter, and it is
        // how Codex works: hooks live in `config.toml` beside the MCP server
        // rather than in a settings file of their own. Nothing is overwritten
        // there — the second render builds on what the first decided — so the
        // question this asks is only ever about two *different* agents.
        for platform in [Platform::Windows, Platform::MacOs, Platform::Unix] {
            let environment = environment(platform);
            let mut owner: BTreeMap<PathBuf, &str> = BTreeMap::new();
            let mut shared_instructions: BTreeMap<PathBuf, Vec<&str>> = BTreeMap::new();
            for adapter in REGISTRY {
                let paths = crate::setup::resolve_paths(adapter, &environment);
                if let Some(instructions) = paths.instructions.clone() {
                    shared_instructions
                        .entry(instructions)
                        .or_default()
                        .push(adapter.slug);
                }
                for path in [Some(paths.mcp_config), paths.hooks].into_iter().flatten() {
                    match owner.insert(path.clone(), adapter.slug) {
                        Some(other) if other != adapter.slug => panic!(
                            "{} and {} both claim {} on {platform:?}",
                            other,
                            adapter.slug,
                            path.display()
                        ),
                        _ => {}
                    }
                }
            }

            for (path, mut agents) in shared_instructions {
                agents.dedup();
                if agents.len() > 1 {
                    assert_eq!(
                        agents,
                        ["gemini-cli", "antigravity"],
                        "{} is shared on {platform:?}",
                        path.display()
                    );
                }
                if let Some(other) = owner.get(&path) {
                    panic!(
                        "{} keeps instructions in {}, which {other} configures",
                        agents.join(" and "),
                        path.display()
                    );
                }
            }
        }
    }

    #[test]
    fn every_agent_answers_every_question_on_every_platform() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Unix] {
            let environment = environment(platform);
            for adapter in REGISTRY {
                let paths = crate::setup::resolve_paths(adapter, &environment);
                assert!(
                    paths.mcp_config.is_absolute(),
                    "{} on {platform:?}: {}",
                    adapter.slug,
                    paths.mcp_config.display()
                );
                if let Some(instructions) = &paths.instructions {
                    assert!(
                        instructions.is_absolute(),
                        "{} on {platform:?}: {}",
                        adapter.slug,
                        instructions.display()
                    );
                }
                assert_eq!(
                    paths.hooks.is_some(),
                    adapter.supports_hooks(),
                    "{} promises hooks it has no file for",
                    adapter.slug
                );
            }
        }
    }

    #[test]
    fn only_a_new_instruction_file_gets_a_preamble() {
        let copilot = REGISTRY
            .iter()
            .find(|adapter| adapter.slug == "vscode-copilot")
            .expect("VS Code Copilot is in the registry");
        assert!(copilot.new_instruction_file.contains("applyTo"));
        for adapter in REGISTRY {
            if adapter.slug != "vscode-copilot" {
                assert_eq!(
                    adapter.new_instruction_file, "",
                    "{} grew a preamble nobody asked for",
                    adapter.slug
                );
            }
        }
    }
}
