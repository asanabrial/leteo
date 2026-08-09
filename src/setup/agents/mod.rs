//! One module per coding agent Leteo can configure.
//!
//! # Why a module each
//!
//! Everything Leteo knew about one agent used to be spread over six places in
//! two files: an entry in the registry table, a `ConfigPath` variant, an
//! `InstructionPath` variant, two `match` arms in the renderer, and the
//! `supports_hooks` predicate. Nothing tied them together, so each could be
//! written without the others and the compiler only caught two of the six — the
//! `match` arms. Adding Antigravity meant finding all six by memory.
//!
//! Now an agent is one file that says where its configuration lives, where its
//! instructions go, and whether it takes hooks. Adding one is writing that file
//! and naming it in [`REGISTRY`]; forgetting a piece is a missing field, which
//! does not compile.
//!
//! # What stays out
//!
//! Rendering. `ConfigFormat::CodexToml` and the JSONC tolerance OpenCode needs
//! are properties of a *file format*, not of an agent — two agents could share
//! either. They stay with the renderer, where the dispatch is already on the
//! format, rather than being pulled in here so that every agent's file would
//! look busier.

use std::path::{Path, PathBuf};

use super::{ConfigFormat, McpFormat, SetupEnvironment};

mod antigravity;
mod claude_code;
mod codex;
mod cursor;
mod gemini_cli;
mod kilocode;
mod kiro;
mod opencode;
mod pi;
mod qwen;
mod vscode_copilot;
mod windsurf;

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
    /// Where this agent keeps its list of MCP servers.
    pub(super) config_path: fn(&SetupEnvironment) -> PathBuf,
    /// Where its instruction file goes.
    ///
    /// Handed the MCP configuration path that was just resolved, because most
    /// agents keep the two side by side and deriving one from the other is
    /// shorter and harder to get wrong than spelling the directory out twice.
    ///
    /// `None` for an agent that reads no instruction file at all.
    pub(super) instruction_path: Option<fn(&SetupEnvironment, &Path) -> PathBuf>,
    /// What a brand-new instruction file starts with, before Leteo's protocol
    /// block is appended to it.
    ///
    /// Empty for all but one agent, and stated by every agent anyway: a default
    /// would let the next one inherit silence, and the one time this matters it
    /// fails invisibly — VS Code Copilot applies an instruction file only if it
    /// carries `applyTo` front matter, so a file written without one is created
    /// successfully and then ignored.
    pub(super) new_instruction_file: &'static str,
    /// Whether that file is one Leteo invented rather than one the agent had.
    ///
    /// Eight agents keep their instructions somewhere that was already theirs —
    /// `CLAUDE.md`, `AGENTS.md` — and uninstalling takes Leteo's block out and
    /// leaves the document alone. Three get a file named after Leteo, and there
    /// taking the block out leaves an empty file with Leteo's name on it, which
    /// `uninstall` promises not to do. For VS Code Copilot it left the `applyTo`
    /// front matter as well: an instruction file that still applies to every
    /// source file and says nothing.
    ///
    /// Stated per agent rather than guessed from the file name, for the reason
    /// `new_instruction_file` is: a default would let the next one inherit the
    /// wrong answer silently.
    pub(super) owns_instruction_file: bool,
    /// Where its lifecycle hooks are registered.
    ///
    /// `None` is the common case. Only an agent with a stable, documented hook
    /// settings file gets one; the rest stay out rather than risk writing
    /// entries the agent would reject.
    pub(super) hooks_path: Option<fn(&SetupEnvironment) -> PathBuf>,
}

impl AgentAdapter {
    /// Whether Leteo can install lifecycle hooks for this agent.
    ///
    /// Callers that offer hooks as a choice need to know this up front: asking
    /// for them on an agent that has none is an error, so the question should
    /// not be put in the first place.
    pub fn supports_hooks(&self) -> bool {
        self.hooks_path.is_some()
    }
}

/// The agents Leteo can configure, in the order they are offered.
pub const REGISTRY: &[AgentAdapter] = &[
    opencode::ADAPTER,
    claude_code::ADAPTER,
    gemini_cli::ADAPTER,
    codex::ADAPTER,
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

    /// The three-way split the prose quotes, taken from the registry.
    ///
    /// `uninstall` behaves differently in each of the three, so the counts are
    /// worth stating — and they were stated by hand in four places, which is how
    /// they went wrong. Pi was added with no instruction file at all and nobody
    /// went back: the doc comment above, `cli.md` §5 and the uninstall guard all
    /// said nine agents kept a file that was already theirs, and eight did.
    ///
    /// A thirteenth agent fails this rather than quietly making four sentences
    /// false, and the numbers to fix are here.
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
            8,
            "keep a file that was already theirs: {theirs:?}"
        );
        assert_eq!(
            ours,
            ["cursor", "vscode-copilot", "kiro"],
            "get a file of Leteo's"
        );
        assert_eq!(none, ["pi"], "read no instruction file at all");
        assert_eq!(theirs.len() + ours.len() + none.len(), REGISTRY.len());

        // And the other count the prose leans on: hooks. Three agents run them,
        // and only two are reached by `setup --hooks` — OpenCode's come from the
        // plugin under `plugin/opencode/`, which maps its lifecycle events onto
        // `leteo hook <event>` without any settings file to write.
        let with_hooks: Vec<_> = REGISTRY
            .iter()
            .filter(|adapter| adapter.supports_hooks())
            .map(|adapter| adapter.slug)
            .collect();
        assert_eq!(
            with_hooks,
            ["claude-code", "codex"],
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

            // An instruction file may be shared, but only where both products
            // genuinely read it. A new pair turning up here is a question to
            // answer in their sources, not a line to add.
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
        // The point of one module per agent: a half-written one cannot compile.
        // What the compiler cannot check is that the paths are usable, so that
        // is checked here rather than found by a person whose setup wrote a
        // file to a relative path from wherever they happened to be standing.
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
        // The one agent that needs one needs it *only* when creating the file.
        // Prepending it to an existing file would put front matter in the
        // middle of somebody's notes.
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
