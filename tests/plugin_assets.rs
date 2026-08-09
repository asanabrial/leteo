//! Checks the shipped plugin bundles against the binary they drive.
//!
//! These files are data, so nothing else would catch a hook event that was
//! renamed, a manifest that stopped being valid JSON, or a marketplace entry
//! pointing at a directory that no longer exists.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Every event `leteo hook` accepts, as declared by the CLI.
const HOOK_EVENTS: &[&str] = &[
    "session-start",
    "post-compaction",
    "user-prompt-submit",
    "subagent-stop",
    "session-stop",
];

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_json(path: &Path) -> Value {
    let content = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&content)
        .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()))
}

/// Walks every `command` string inside a hooks manifest.
fn hook_commands(manifest: &Value) -> Vec<String> {
    let mut commands = Vec::new();
    let events = manifest["hooks"]
        .as_object()
        .expect("a hooks manifest has a hooks object");
    for (event, entries) in events {
        let entries = entries
            .as_array()
            .unwrap_or_else(|| panic!("hooks.{event} must be an array"));
        for entry in entries {
            let inner = entry["hooks"]
                .as_array()
                .unwrap_or_else(|| panic!("hooks.{event}[].hooks must be an array"));
            for hook in inner {
                assert_eq!(
                    hook["type"].as_str(),
                    Some("command"),
                    "hooks.{event} must run a command"
                );
                commands.push(
                    hook["command"]
                        .as_str()
                        .unwrap_or_else(|| panic!("hooks.{event} needs a command string"))
                        .to_owned(),
                );
            }
        }
    }
    commands
}

#[test]
fn every_plugin_hook_invokes_an_event_the_binary_accepts() {
    for bundle in ["claude-code", "codex"] {
        let path = repository_root()
            .join("plugin")
            .join(bundle)
            .join("hooks/hooks.json");
        let commands = hook_commands(&read_json(&path));
        assert_eq!(
            commands.len(),
            HOOK_EVENTS.len(),
            "{bundle} should register one hook per lifecycle event: {commands:?}"
        );
        for command in &commands {
            let event = command
                .strip_prefix("leteo hook ")
                .unwrap_or_else(|| panic!("{bundle}: {command:?} must call `leteo hook <event>`"));
            assert!(
                HOOK_EVENTS.contains(&event),
                "{bundle}: {event:?} is not an event the CLI accepts"
            );
        }
        // The whole point of the in-binary hooks is that a bundle needs no
        // shell scripts, so a stray script directory means the port regressed.
        let scripts = repository_root()
            .join("plugin")
            .join(bundle)
            .join("scripts");
        assert!(
            !scripts.exists(),
            "{bundle} should drive the binary directly instead of shipping scripts"
        );
    }
}

#[test]
fn every_plugin_registers_the_same_mcp_server_the_setup_command_does() {
    for bundle in ["claude-code", "codex"] {
        let path = repository_root()
            .join("plugin")
            .join(bundle)
            .join(".mcp.json");
        let manifest = read_json(&path);
        let server = &manifest["mcpServers"]["leteo"];
        assert_eq!(
            server["command"].as_str(),
            Some("leteo"),
            "{bundle} must launch the leteo binary"
        );
        assert_eq!(
            server["args"],
            serde_json::json!(["mcp", "--tools=agent"]),
            "{bundle} must request the agent tool profile"
        );
    }
}

#[test]
fn plugin_manifests_declare_the_crate_version_and_license() {
    let expected = env!("CARGO_PKG_VERSION");
    for (bundle, manifest) in [
        ("claude-code", ".claude-plugin/plugin.json"),
        ("codex", ".codex-plugin/plugin.json"),
    ] {
        let path = repository_root().join("plugin").join(bundle).join(manifest);
        let manifest = read_json(&path);
        assert_eq!(manifest["name"].as_str(), Some("leteo"));
        assert_eq!(
            manifest["version"].as_str(),
            Some(expected),
            "{bundle} version drifted from Cargo.toml"
        );
        assert_eq!(manifest["license"].as_str(), Some("MIT"));
    }
}

#[test]
fn marketplace_entries_point_at_bundles_that_exist() {
    let root = repository_root();
    let claude = read_json(&root.join(".claude-plugin/marketplace.json"));
    let source = claude["plugins"][0]["source"]
        .as_str()
        .expect("the Claude marketplace entry names a source");
    assert!(
        root.join(source.trim_start_matches("./")).is_dir(),
        "the Claude marketplace points at {source}, which does not exist"
    );
    assert_eq!(
        claude["plugins"][0]["version"].as_str(),
        Some(env!("CARGO_PKG_VERSION"))
    );

    let codex = read_json(&root.join(".agents/plugins/marketplace.json"));
    let source = codex["plugins"][0]["source"]["path"]
        .as_str()
        .expect("the Codex marketplace entry names a path");
    assert!(
        root.join(source.trim_start_matches("./")).is_dir(),
        "the Codex marketplace points at {source}, which does not exist"
    );
}

#[test]
fn every_bundle_ships_a_memory_skill_with_frontmatter() {
    for bundle in ["claude-code", "codex"] {
        let path = repository_root()
            .join("plugin")
            .join(bundle)
            .join("skills/memory/SKILL.md");
        let skill = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            skill.starts_with("---\n"),
            "{bundle} skill needs YAML frontmatter"
        );
        assert!(
            skill.contains("name: leteo-memory"),
            "{bundle} skill must be named leteo-memory"
        );
        // The protocol is worthless if it does not name the tool to call.
        assert!(
            skill.contains("mem_save"),
            "{bundle} skill must teach saving"
        );
        assert!(
            !skill.contains("engram"),
            "{bundle} skill still refers to the upstream project"
        );
        // Sardi is how Leteo speaks to a person. Agents only learn that from
        // here, so a bundle that drops it silently loses the voice.
        assert!(
            skill.contains("Sardi"),
            "{bundle} skill must teach how to report memory work"
        );
        assert!(
            skill.contains("Never in an error"),
            "{bundle} skill must keep failures free of the mascot"
        );
    }
}

#[test]
fn the_two_memory_skills_differ_only_in_the_setup_command() {
    // They are the same protocol, kept as two files because each bundle ships
    // its own. Editing one and forgetting the other is the obvious failure,
    // and nothing else would notice.
    let read = |bundle: &str| {
        let path = repository_root()
            .join("plugin")
            .join(bundle)
            .join("skills/memory/SKILL.md");
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    };
    let claude = read("claude-code");
    let codex = read("codex");
    assert_eq!(
        claude.replace("leteo setup claude-code", "leteo setup <agent>"),
        codex.replace("leteo setup codex", "leteo setup <agent>"),
        "the two memory skills have drifted apart"
    );
}

/// Every `mem_…` name a document mentions.
fn tools_named_in(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes = text.as_bytes();
    for (start, _) in text.match_indices("mem_") {
        // A name preceded by a word character is part of something else.
        if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            continue;
        }
        let name: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || *c == '_')
            .collect();
        let name = name.trim_end_matches('_').to_owned();
        if name.len() > 4 {
            found.insert(name);
        }
    }
    found
}

#[test]
fn the_skill_names_the_tools_the_binary_has_and_no_others() {
    // The skill is the one document agents read and obey. A tool renamed
    // leaves it teaching a call that fails; a tool added and never written up
    // is one no agent learns to use. Neither is a compile error and neither
    // shows in any other test — the shipped file is data.
    let declared = tools_named_in(
        &std::fs::read_to_string(repository_root().join("src/mcp/tools.rs"))
            .expect("read tools.rs"),
    );
    assert!(
        declared.len() > 15,
        "the scanner found only {declared:?}, so it has stopped matching tools.rs"
    );
    // Every tool an agent is given by default has to be taught.
    let offered: BTreeSet<String> = leteo::mcp::PROFILE_AGENT
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect();

    for bundle in ["claude-code", "codex"] {
        let path = repository_root()
            .join("plugin")
            .join(bundle)
            .join("skills/memory/SKILL.md");
        let named = tools_named_in(
            &std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
        );

        let phantom: Vec<&String> = named.difference(&declared).collect();
        assert!(
            phantom.is_empty(),
            "{bundle} skill tells agents to call tools that do not exist: {phantom:?}"
        );
        let untaught: Vec<&String> = offered.difference(&named).collect();
        assert!(
            untaught.is_empty(),
            "{bundle} skill never mentions these, so no agent learns they exist: {untaught:?}"
        );

        // And the count it gives for the deferred ones is that many.
        //
        // The skill says "only the three that change or count the whole store
        // are deferred" and then names them. Three is a hand-written number
        // beside a list the binary owns: a fourth admin tool would be named in
        // that sentence, pass the two checks above because it is mentioned,
        // and leave the word wrong on the page every agent reads first.
        let deferred = leteo::mcp::PROFILE_ADMIN.len();
        let spelled = match deferred {
            2 => "two",
            3 => "three",
            4 => "four",
            other => panic!("nobody has written the word for {other} deferred tools"),
        };
        let text = std::fs::read_to_string(&path).expect("read the skill again");
        assert!(
            text.contains(&format!("Only the {spelled} that")),
            "{bundle} skill counts the deferred tools by hand and there are {deferred}"
        );
        for tool in leteo::mcp::PROFILE_ADMIN {
            assert!(
                text.contains(tool),
                "{bundle} skill never names the deferred tool {tool}"
            );
        }
    }
}

/// The slice of a document that declares the memory kinds.
///
/// Checking the whole file for each word is not the same question, and it
/// answers yes for the wrong reason: the skill says "The user states a
/// preference" in prose, so `preference` could vanish from the type list
/// entirely and a whole-document search would still find it. That is what this
/// guard did until it was broken on purpose and failed to notice.
fn kind_declaration(text: &str, opening: &str) -> String {
    let start = text
        .find(opening)
        .unwrap_or_else(|| panic!("no kind declaration starting {opening:?}"));
    let rest = &text[start + opening.len()..];
    // Up to the next bullet or blank line, so the wrapped second line comes
    // along and nothing after it does.
    let end = rest
        .find(
            "
- ",
        )
        .or_else(|| {
            rest.find(
                "

",
            )
        })
        .unwrap_or(rest.len());
    rest[..end].to_owned()
}

#[test]
fn the_skill_teaches_the_kinds_the_tool_schema_asks_for() {
    // The seven kinds lived only as prose, in the skill and in the `type`
    // field's own documentation, with nothing tying either to the code. An
    // agent taught a kind the schema does not name files memories where the
    // `type` filter never looks.
    let schema = std::fs::read_to_string(repository_root().join("src/mcp/params.rs"))
        .expect("read params.rs");
    let offered = kind_declaration(&schema, "/// One of:");

    for bundle in ["claude-code", "codex"] {
        let path = repository_root()
            .join("plugin")
            .join(bundle)
            .join("skills/memory/SKILL.md");
        let skill = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let taught = kind_declaration(&skill, "- **type**:");

        for kind in leteo::memory::rules::KINDS {
            assert!(
                taught.contains(&format!("`{kind}`")),
                "{bundle} skill's type list does not offer {kind}: {taught:?}"
            );
            assert!(
                offered.contains(kind),
                "the mem_save schema's type list does not offer {kind}: {offered:?}"
            );
        }

        // The verdicts are the same shape again, and split rather than listed:
        // the skill sorts them into the ones an agent settles itself and the
        // ones it puts to the user. A seventh would land in neither bucket and
        // an agent would have no instruction for it at all — which reads
        // exactly like a verdict that needs no decision.
        for verb in leteo::memory::rules::RELATION_VERBS {
            assert!(
                skill.contains(&format!("`{verb}`")),
                "{bundle} skill sorts the verdicts into two buckets and {verb} is in neither"
            );
        }

        // Scope is the same shape one line below, and it was the one that had
        // drifted: the skill offered `project` and `personal` while the door
        // took a third and `memory-model.md` §11 named three. An agent is
        // taught what to write from this line and from nowhere else.
        let taught_scope = kind_declaration(&skill, "- **scope**:");
        for scope in leteo::memory::normalize::SCOPES {
            assert!(
                taught_scope.contains(&format!("`{scope}`")),
                "{bundle} skill's scope list does not offer {scope}: {taught_scope:?}"
            );
        }
    }
}

#[test]
fn the_skill_says_which_tools_are_there_and_which_need_fetching() {
    // Naming a tool somewhere in the document is not the same as telling an
    // agent it can call it. Five tools of the agent profile were listed under
    // "Admin tools are deferred; reach for `ToolSearch`" — `mem_timeline`,
    // `mem_doctor`, `mem_capture_passive`, `mem_judge` and `mem_compare` are
    // all loaded from the first message. An agent believing that wastes a
    // round-trip at best, and at worst reads `mem_judge` as out of reach while
    // the same skill tells it to judge every candidate.
    //
    // The earlier guard passed through all of it, because it only asked
    // whether each name appeared. This asks where.
    let agent: BTreeSet<String> = leteo::mcp::PROFILE_AGENT
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect();
    let admin: BTreeSet<String> = leteo::mcp::PROFILE_ADMIN
        .iter()
        .map(|tool| (*tool).to_owned())
        .collect();

    for bundle in ["claude-code", "codex"] {
        let path = repository_root()
            .join("plugin")
            .join(bundle)
            .join("skills/memory/SKILL.md");
        let skill = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let split = skill
            .find("are deferred")
            .unwrap_or_else(|| panic!("{bundle} skill no longer says what is deferred"));
        let present = tools_named_in(&skill[..split]);
        // That sentence and no further. Taking the whole rest of the document
        // as "deferred" was the same mistake this test exists to catch: every
        // tool the skill goes on to teach would land in it. `mem_context` did.
        let rest = &skill[split..];
        let paragraph = rest
            .find(
                "

",
            )
            .map_or(rest, |end| &rest[..end]);
        let deferred = tools_named_in(paragraph);

        for tool in &agent {
            assert!(
                present.contains(tool),
                "{bundle} skill does not list {tool} among the tools already there"
            );
            assert!(
                !deferred.contains(tool),
                "{bundle} skill sends an agent to ToolSearch for {tool}, which is already loaded"
            );
        }
        for tool in &admin {
            assert!(
                deferred.contains(tool),
                "{bundle} skill does not say {tool} has to be fetched"
            );
        }
    }
}
