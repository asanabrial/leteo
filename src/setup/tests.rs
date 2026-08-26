use std::collections::BTreeMap;

use super::*;
use tempfile::TempDir;

fn options(temp: &TempDir) -> SetupOptions {
    SetupOptions {
        platform: Some(Platform::Unix),
        home_dir: Some(temp.path().to_owned()),
        config_home: Some(temp.path().join("config")),
        app_data: Some(temp.path().join("appdata")),
        executable: Some(temp.path().join("bin").join("leteo")),
        ..SetupOptions::default()
    }
}

fn write_fixture(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn registry_contains_the_requested_agents() {
    let slugs = supported_agents()
        .iter()
        .map(|adapter| adapter.slug)
        .collect::<Vec<_>>();
    assert_eq!(
        slugs,
        [
            "opencode",
            "claude-code",
            "zcode",
            "gemini-cli",
            "codex",
            "cursor",
            "windsurf",
            "vscode-copilot",
            "kilocode",
            "qwen",
            "kiro",
            "antigravity",
            "pi",
        ]
    );
}

#[test]
fn antigravity_uses_the_shared_gemini_mcp_config_and_context_file() {
    let temp = TempDir::new().unwrap();
    let setup_options = options(&temp);
    let paths = resolve_agent_paths("antigravity", &setup_options).unwrap();
    assert_eq!(
        paths.mcp_config,
        temp.path()
            .join(".gemini")
            .join("config")
            .join("mcp_config.json")
    );
    assert_eq!(
        paths.instructions,
        Some(temp.path().join(".gemini").join("GEMINI.md"))
    );
    // The shared file must stay distinct from the Gemini CLI's own settings.
    let gemini = resolve_agent_paths("gemini-cli", &setup_options).unwrap();
    assert_ne!(paths.mcp_config, gemini.mcp_config);

    let result = setup(
        "antigravity",
        &SetupOptions {
            install_instructions: true,
            ..setup_options
        },
    )
    .unwrap();
    assert_eq!(result.changed_files(), 2);
    let config = read_json(&paths.mcp_config);
    assert_eq!(config["mcpServers"]["leteo"]["args"][0], "mcp");
    let instructions = fs::read_to_string(paths.instructions.unwrap()).unwrap();
    assert!(instructions.contains(MEMORY_PROTOCOL_BEGIN));
}

#[test]
fn pi_registers_the_agent_profile_and_has_no_instruction_file() {
    let temp = TempDir::new().unwrap();
    let setup_options = options(&temp);
    let paths = resolve_agent_paths("pi", &setup_options).unwrap();

    assert_eq!(
        paths.mcp_config,
        temp.path().join(".pi").join("agent").join("mcp.json")
    );
    assert_eq!(paths.instructions, None, "Pi reads no instruction file");
    assert_eq!(paths.hooks, None);

    // A foreign server and unrelated settings must survive.
    write_fixture(
        &paths.mcp_config,
        r#"{"mcpServers":{"other":{"command":"other"}},"theme":"dark"}"#,
    );
    let result = setup("pi", &setup_options).unwrap();
    assert_eq!(result.changed_files(), 1);

    let config = read_json(&paths.mcp_config);
    assert_eq!(config["theme"], "dark");
    assert_eq!(config["mcpServers"]["other"]["command"], "other");
    let leteo = &config["mcpServers"]["leteo"];
    // Pi renders one row per tool, so it gets the agent profile.
    assert_eq!(leteo["args"], json!(["mcp", "--tools=agent"]));
    // `pi-mcp-extension` is what reads this file, and there `lazy` — its own
    // default — holds the server down until somebody types `/mcp:start leteo`.
    // Leteo wrote `lazy`, so its tools were missing from every Pi session
    // nobody turned them on in.
    assert_eq!(leteo["lifecycle"], "eager");

    // Asking for instructions fails loudly instead of writing a file Pi
    // would never read.
    let error = setup(
        "pi",
        &SetupOptions {
            install_instructions: true,
            ..setup_options.clone()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("has no instruction file"), "{error}");

    let second = setup("pi", &setup_options).unwrap();
    assert_eq!(second.changed_files(), 0, "the second run is a no-op");
}

#[test]
fn hooks_are_installed_idempotently_and_preserve_foreign_entries() {
    let temp = TempDir::new().unwrap();
    let setup_options = SetupOptions {
        install_hooks: true,
        ..options(&temp)
    };
    let paths = resolve_agent_paths("claude-code", &setup_options).unwrap();
    let hooks_path = paths.hooks.expect("Claude Code supports hooks");
    write_fixture(
        &hooks_path,
        r#"{
              "model": "opus",
              "hooks": {
                "UserPromptSubmit": [
                  {"hooks": [{"type": "command", "command": "other-tool prompt"}]}
                ],
                "SessionStart": [
                  {"matcher": "startup|clear",
                   "hooks": [{"type": "command", "command": "/old/path/leteo hook session-start"}]}
                ]
              }
            }"#,
    );

    let first = setup("claude-code", &setup_options).unwrap();
    let hook_action = first
        .actions
        .iter()
        .find(|action| action.kind == ActionKind::Hooks)
        .expect("hooks action");
    assert_eq!(hook_action.change, Change::Update);
    assert_eq!(hook_action.path, hooks_path);

    let settings = read_json(&hooks_path);
    assert_eq!(settings["model"], "opus", "unrelated settings survive");
    let prompts = settings["hooks"]["UserPromptSubmit"]
        .as_array()
        .expect("prompt hooks");
    assert_eq!(prompts.len(), 2, "the foreign hook is preserved");
    assert_eq!(prompts[0]["hooks"][0]["command"], "other-tool prompt");
    assert!(
        prompts[1]["hooks"][0]["command"]
            .as_str()
            .expect("leteo command")
            .ends_with("hook user-prompt-submit")
    );
    let session_start = settings["hooks"]["SessionStart"]
        .as_array()
        .expect("session hooks");
    assert_eq!(session_start.len(), 2, "the stale Leteo entry is replaced");
    assert_eq!(session_start[0]["matcher"], "startup|clear");
    assert_eq!(session_start[1]["matcher"], "compact");
    assert!(session_start.iter().all(|entry| {
        !entry["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .starts_with("/old/path/")
    }));
    assert_eq!(settings["hooks"]["SessionEnd"][0]["hooks"][0]["timeout"], 3);

    let second = setup("claude-code", &setup_options).unwrap();
    assert!(
        second
            .actions
            .iter()
            .all(|action| action.change == Change::Unchanged),
        "a second run rewrites nothing: {:?}",
        second.actions
    );
}

#[test]
fn another_tool_using_the_same_event_names_keeps_its_hooks() {
    // Found on a real machine, and the reason this is a test rather than a
    // comment: `warden` registers `hook session-start` and `hook session-end`
    // in the same `settings.json`, because those are the names the events
    // have. Leteo recognized its own entries by the subcommand alone, so every
    // one of them read as Leteo's.
    //
    // What that costs, in the three places that asked:
    //   - installing hooks **deleted** them, on the way to writing its own;
    //   - uninstalling deleted them and left nothing behind to say so;
    //   - `leteo doctor` reported "Claude Code runs every Leteo hook twice"
    //     and told the person to remove hooks Leteo had never installed.
    //
    // A tool that eats another tool's configuration is the complaint that
    // started the uninstaller being written in the first place. The subcommand
    // is a name anybody may use; the binary in front of it is the identity.
    let temp = TempDir::new().unwrap();
    let setup_options = SetupOptions {
        install_hooks: true,
        ..options(&temp)
    };
    let paths = resolve_agent_paths("claude-code", &setup_options).unwrap();
    let hooks_path = paths.hooks.expect("Claude Code supports hooks");
    // Written through `json!` rather than as a literal: the command is a quoted
    // Windows path inside a JSON string, and hand-escaping it is how a fixture
    // ends up testing its own backslashes.
    let foreign = "\"C:\\Users\\me\\.cargo\\bin\\warden.exe\" hook session-start";
    write_fixture(
        &hooks_path,
        &serde_json::json!({
            "hooks": {
                "SessionStart": [
                    {"hooks": [{"type": "command", "command": foreign}]}
                ],
                "SessionEnd": [
                    {"hooks": [{"type": "command", "command": "warden hook session-stop"}]}
                ]
            }
        })
        .to_string(),
    );

    setup("claude-code", &setup_options).unwrap();

    let settings = read_json(&hooks_path);
    let commands = |event: &str| -> Vec<String> {
        settings["hooks"][event]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .flat_map(|entry| {
                entry["hooks"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
            })
            .filter_map(|hook| hook["command"].as_str().map(str::to_owned))
            .collect()
    };
    assert!(
        commands("SessionStart")
            .iter()
            .any(|command| command.contains("warden")),
        "the other tool's SessionStart hook was deleted: {:?}",
        commands("SessionStart")
    );
    assert!(
        commands("SessionEnd")
            .iter()
            .any(|command| command.contains("warden")),
        "the other tool's SessionEnd hook was deleted: {:?}",
        commands("SessionEnd")
    );

    // And Leteo's own still went in beside them, so this refuses what belongs
    // to somebody else rather than refusing to install.
    assert!(
        commands("SessionStart")
            .iter()
            .any(|command| command.ends_with("hook session-start") && !command.contains("warden"))
    );

    // Taking Leteo out again leaves the other tool exactly as it was.
    let removal = SetupOptions {
        install_hooks: true,
        ..options(&temp)
    };
    uninstall("claude-code", &removal).unwrap();
    let settings = read_json(&hooks_path);
    let survivors: Vec<String> = ["SessionStart", "SessionEnd"]
        .iter()
        .flat_map(|event| {
            settings["hooks"][*event]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|entry| entry["hooks"][0]["command"].as_str().map(str::to_owned))
        .collect();
    assert_eq!(
        survivors.len(),
        2,
        "uninstall took the other tool's hooks with it: {survivors:?}"
    );
    assert!(survivors.iter().all(|command| command.contains("warden")));
}

#[test]
fn hook_installation_is_refused_for_agents_without_a_known_format() {
    let temp = TempDir::new().unwrap();
    let setup_options = SetupOptions {
        install_hooks: true,
        ..options(&temp)
    };
    assert!(
        resolve_agent_paths("cursor", &setup_options)
            .unwrap()
            .hooks
            .is_none()
    );
    let error = setup("cursor", &setup_options).unwrap_err().to_string();
    assert!(error.contains("does not support Leteo lifecycle hooks"));
}

#[test]
fn generated_commands_never_contain_a_windows_verbatim_path() {
    let temp = TempDir::new().unwrap();
    // A real, canonicalizable executable path: canonicalize() only returns
    // the `\\?\` prefix for paths that exist.
    let executable = temp.path().join("leteo.exe");
    fs::write(&executable, b"binary").unwrap();
    let setup_options = SetupOptions {
        install_hooks: true,
        install_instructions: false,
        executable: Some(executable),
        ..options(&temp)
    };
    let paths = resolve_agent_paths("claude-code", &setup_options).unwrap();

    setup("claude-code", &setup_options).unwrap();

    let config = fs::read_to_string(&paths.mcp_config).unwrap();
    let hooks = fs::read_to_string(paths.hooks.unwrap()).unwrap();
    assert!(!config.contains(r"\\?\"), "MCP config: {config}");
    assert!(!hooks.contains(r"\\?\"), "hooks: {hooks}");
    assert!(hooks.contains("hook session-start"));
}

#[test]
fn hook_commands_quote_a_path_with_no_space_in_it() {
    // The failure this guards is the ordinary install path, not an exotic
    // one: hooks run through bash, bash reads a backslash as an escape, and
    // an unquoted `C:\Users\me\AppData\Local\leteo\bin\leteo.exe` reaches it
    // as `C:UsersmeAppDataLocalleteobinleteo.exe`. Quoting was conditional
    // on a space, so every default Windows install failed with "command not
    // found" on every prompt.
    let temp = TempDir::new().unwrap();
    let plain = temp.path().join("AppData").join("Local").join("leteo");
    fs::create_dir_all(&plain).unwrap();
    let executable = plain.join("leteo.exe");
    fs::write(&executable, b"binary").unwrap();
    let setup_options = SetupOptions {
        install_hooks: true,
        executable: Some(executable),
        ..options(&temp)
    };
    let paths = resolve_agent_paths("claude-code", &setup_options).unwrap();

    setup("claude-code", &setup_options).unwrap();

    let settings = read_json(&paths.hooks.unwrap());
    for event in [
        "SessionStart",
        "SessionEnd",
        "SubagentStop",
        "UserPromptSubmit",
    ] {
        let command = settings["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("{event} hook command"));
        assert!(
            command.starts_with('"') && command[1..].contains("\" hook "),
            "{event}: {command}"
        );
    }
}

#[test]
fn a_hook_that_changed_events_leaves_nothing_where_it_used_to_be() {
    // `session-stop` used to sit on `Stop`, which fires at the end of every
    // turn rather than at the end of the conversation. Every machine that ran
    // an older setup still holds that entry, and pruning only the events about
    // to be written would leave it there firing forever beside its replacement.
    let temp = TempDir::new().unwrap();
    let setup_options = SetupOptions {
        install_hooks: true,
        ..options(&temp)
    };
    let paths = resolve_agent_paths("claude-code", &setup_options).unwrap();
    let hooks_path = paths.hooks.unwrap();
    write_fixture(
        &hooks_path,
        r#"{"hooks":{
             "Stop":[{"hooks":[{"type":"command","command":"/old/leteo hook session-stop"}]}],
             "PreToolUse":[
               {"hooks":[{"type":"command","command":"/old/leteo hook session-stop"}]},
               {"hooks":[{"type":"command","command":"someone-else guard"}]}
             ]
           }}"#,
    );

    setup("claude-code", &setup_options).unwrap();

    let settings = read_json(&hooks_path);
    assert!(
        settings["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
            .as_str()
            .is_some_and(|command| command.ends_with("\" hook session-stop")),
        "the hook should be registered on its new event: {settings}"
    );
    assert!(
        settings["hooks"].get("Stop").is_none(),
        "an event holding nothing but Leteo's leftover should go with it: {settings}"
    );
    // Somebody else's event is a different matter: Leteo's entry comes out of
    // it, and the event stays because it was never Leteo's to remove.
    let guarded = settings["hooks"]["PreToolUse"]
        .as_array()
        .expect("the foreign event survives");
    assert_eq!(guarded.len(), 1, "{settings}");
    assert_eq!(guarded[0]["hooks"][0]["command"], "someone-else guard");
}

#[test]
fn hook_commands_quote_an_executable_path_containing_spaces() {
    let temp = TempDir::new().unwrap();
    let program_files = temp.path().join("Program Files").join("leteo");
    fs::create_dir_all(&program_files).unwrap();
    let executable = program_files.join("leteo.exe");
    fs::write(&executable, b"binary").unwrap();
    let setup_options = SetupOptions {
        install_hooks: true,
        executable: Some(executable),
        ..options(&temp)
    };
    let paths = resolve_agent_paths("claude-code", &setup_options).unwrap();

    setup("claude-code", &setup_options).unwrap();

    let settings = read_json(&paths.hooks.unwrap());
    let command = settings["hooks"]["SessionEnd"][0]["hooks"][0]["command"]
        .as_str()
        .expect("session end hook command");
    assert!(command.starts_with('"'), "{command}");
    assert!(command.ends_with("\" hook session-stop"), "{command}");

    // The MCP entry is argv[0], never a shell string, so it stays unquoted.
    let config = read_json(&paths.mcp_config);
    let mcp_command = config["mcpServers"]["leteo"]["command"]
        .as_str()
        .expect("mcp command");
    assert!(!mcp_command.starts_with('"'), "{mcp_command}");
}

#[test]
fn hooks_are_not_touched_unless_requested() {
    let temp = TempDir::new().unwrap();
    let setup_options = options(&temp);
    let paths = resolve_agent_paths("claude-code", &setup_options).unwrap();

    let result = setup("claude-code", &setup_options).unwrap();

    assert!(
        result
            .actions
            .iter()
            .all(|action| action.kind != ActionKind::Hooks)
    );
    assert!(!paths.hooks.expect("hooks path").exists());
}

#[test]
fn fixtures_cover_all_json_formats_and_preserve_existing_values() {
    struct Fixture {
        agent: &'static str,
        top_key: &'static str,
        expected_type: Option<&'static str>,
        command_is_array: bool,
    }

    let fixtures = [
        Fixture {
            agent: "cursor",
            top_key: "mcpServers",
            expected_type: None,
            command_is_array: false,
        },
        Fixture {
            agent: "vscode-copilot",
            top_key: "servers",
            expected_type: Some("stdio"),
            command_is_array: false,
        },
        Fixture {
            agent: "opencode",
            top_key: "mcp",
            expected_type: Some("local"),
            command_is_array: true,
        },
    ];

    for fixture in fixtures {
        let temp = TempDir::new().unwrap();
        let setup_options = options(&temp);
        let paths = resolve_agent_paths(fixture.agent, &setup_options).unwrap();
        let seed = format!(
            r#"{{"theme":"dark","{}":{{"other":{{"command":"other","args":["serve"]}},"leteo":{{"command":"stale"}}}}}}"#,
            fixture.top_key
        );
        write_fixture(&paths.mcp_config, &seed);

        let first = setup(fixture.agent, &setup_options).unwrap();
        assert_eq!(first.changed_files(), 1, "{} first run", fixture.agent);
        let first_bytes = fs::read(&paths.mcp_config).unwrap();
        let config = read_json(&paths.mcp_config);
        assert_eq!(config["theme"], "dark");
        assert_eq!(config[fixture.top_key]["other"]["command"], "other");

        let leteo = &config[fixture.top_key][SERVER_NAME];
        assert_eq!(leteo["type"].as_str(), fixture.expected_type);
        assert_eq!(leteo["command"].is_array(), fixture.command_is_array);
        // Every format asks for the same tool profile, so installing
        // through setup and installing the plugin bundle give an agent the
        // same set rather than differing by route.
        if fixture.command_is_array {
            assert_eq!(leteo["command"][1], "mcp");
            assert_eq!(leteo["command"][2], "--tools=agent");
            assert_eq!(leteo["enabled"], true);
        } else {
            assert_eq!(leteo["args"], json!(["mcp", "--tools=agent"]));
        }

        let second = setup(fixture.agent, &setup_options).unwrap();
        assert_eq!(second.changed_files(), 0, "{} second run", fixture.agent);
        assert_eq!(fs::read(&paths.mcp_config).unwrap(), first_bytes);
    }
}

#[test]
fn opencode_jsonc_accepts_comments_and_trailing_commas() {
    let temp = TempDir::new().unwrap();
    let setup_options = options(&temp);
    let jsonc_path = setup_options
        .config_home
        .as_ref()
        .unwrap()
        .join("opencode")
        .join("opencode.jsonc");
    write_fixture(
        &jsonc_path,
        r#"{
                // OpenCode permits JSONC.
                "theme": "dark",
                "mcp": {
                    "other": {
                        "type": "local",
                        "command": ["other", "mcp"],
                    },
                },
            }"#,
    );
    let paths = resolve_agent_paths("opencode", &setup_options).unwrap();
    assert_eq!(paths.mcp_config, jsonc_path);

    setup("opencode", &setup_options).unwrap();
    let config = read_json(&paths.mcp_config);
    assert_eq!(config["theme"], "dark");
    assert_eq!(config["mcp"]["other"]["command"][0], "other");
    assert_eq!(config["mcp"][SERVER_NAME]["type"], "local");
    assert_eq!(config["mcp"][SERVER_NAME]["command"][1], "mcp");
}

#[test]
fn marker_protocol_is_idempotent_and_preserves_user_instructions() {
    let temp = TempDir::new().unwrap();
    let mut setup_options = options(&temp);
    setup_options.install_instructions = true;
    let paths = resolve_agent_paths("kilocode", &setup_options).unwrap();
    let instructions = paths.instructions.expect("Kilo Code reads instructions");
    write_fixture(&instructions, "# Personal rules\n\nKeep this text.\n");

    setup("kilocode", &setup_options).unwrap();
    let first = fs::read_to_string(&instructions).unwrap();
    setup("kilocode", &setup_options).unwrap();
    let second = fs::read_to_string(&instructions).unwrap();

    assert_eq!(first, second);
    assert!(second.contains("Keep this text."));
    assert_eq!(second.matches(MEMORY_PROTOCOL_BEGIN).count(), 1);
    assert_eq!(second.matches(MEMORY_PROTOCOL_END).count(), 1);
    assert!(second.contains("Leteo Persistent Memory"));
}

#[test]
fn dry_run_reports_changes_without_touching_the_filesystem() {
    let temp = TempDir::new().unwrap();
    let mut setup_options = options(&temp);
    setup_options.dry_run = true;
    setup_options.install_instructions = true;
    let paths = resolve_agent_paths("kiro", &setup_options).unwrap();

    let result = setup("kiro", &setup_options).unwrap();

    assert_eq!(result.changed_files(), 2);
    assert_eq!(result.actions[0].change, Change::Create);
    assert_eq!(result.actions[1].change, Change::Create);
    assert!(!paths.mcp_config.exists());
    assert!(
        !paths
            .instructions
            .expect("Kiro reads instructions")
            .exists()
    );
}

#[test]
fn codex_toml_updates_only_leteo_and_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let setup_options = options(&temp);
    let paths = resolve_agent_paths("codex", &setup_options).unwrap();
    write_fixture(
        &paths.mcp_config,
        r#"model = "gpt-5"

[mcp_servers.other]
command = "other"
args = ["serve"]

[mcp_servers.leteo]
command = "stale"
args = ["old"]
"#,
    );

    setup("codex", &setup_options).unwrap();
    let first = fs::read_to_string(&paths.mcp_config).unwrap();
    setup("codex", &setup_options).unwrap();
    let second = fs::read_to_string(&paths.mcp_config).unwrap();

    assert_eq!(first, second);
    assert!(second.contains("model = \"gpt-5\""));
    assert!(second.contains("[mcp_servers.other]"));
    assert!(second.contains("command = \"other\""));
    assert_eq!(second.matches("[mcp_servers.leteo]").count(), 1);
    assert!(second.contains("args = [\"mcp\", \"--tools=agent\"]"));
    assert!(!second.contains("command = \"stale\""));
}

#[test]
fn uninstalling_takes_leteo_out_and_leaves_everything_else() {
    // These files hold other people's servers, other tools' hooks and
    // somebody's own notes. Removing Leteo must cost them none of it, so
    // this asserts what survives rather than only what goes.
    let temp = TempDir::new().unwrap();
    let setup_options = SetupOptions {
        install_hooks: true,
        install_instructions: true,
        ..options(&temp)
    };
    let paths = resolve_agent_paths("claude-code", &setup_options).unwrap();
    let hooks_path = paths.hooks.clone().unwrap();
    let instructions_path = paths.instructions.clone().unwrap();

    write_fixture(
        &paths.mcp_config,
        r#"{"mcpServers":{"other":{"command":"other"}},"theme":"dark"}"#,
    );
    write_fixture(
        &hooks_path,
        r#"{"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"codegraph prompt-hook"}]}]},"permissions":{"allow":[]}}"#,
    );
    write_fixture(&instructions_path, "# My own notes\n\nKeep these.\n");

    setup("claude-code", &setup_options).unwrap();
    let installed = fs::read_to_string(&paths.mcp_config).unwrap();
    assert!(installed.contains("\"leteo\""), "{installed}");

    let result = uninstall("claude-code", &setup_options).unwrap();
    assert_eq!(result.agent, "claude-code");

    let config = fs::read_to_string(&paths.mcp_config).unwrap();
    assert!(!config.contains("\"leteo\""), "leteo has to go: {config}");
    assert!(
        config.contains("\"other\""),
        "the other server stays: {config}"
    );
    assert!(
        config.contains("\"theme\""),
        "and so does the rest: {config}"
    );

    let hooks = fs::read_to_string(&hooks_path).unwrap();
    assert!(!hooks.contains("hook session-start"), "{hooks}");
    assert!(
        hooks.contains("codegraph prompt-hook"),
        "another tool's hook must survive: {hooks}"
    );
    assert!(hooks.contains("permissions"), "{hooks}");

    let instructions = fs::read_to_string(&instructions_path).unwrap();
    assert!(
        !instructions.contains(MEMORY_PROTOCOL_END),
        "{instructions}"
    );
    assert!(
        instructions.contains("Keep these."),
        "somebody's own notes must survive: {instructions}"
    );

    // And doing it twice is not an error, nor a second round of changes.
    let again = uninstall("claude-code", &setup_options).unwrap();
    assert_eq!(again.changed_files(), 0, "{:?}", again.actions);
}

#[test]
fn uninstalling_an_agent_that_never_had_leteo_changes_nothing() {
    let temp = TempDir::new().unwrap();
    let setup_options = options(&temp);
    let result = uninstall("cursor", &setup_options).unwrap();
    assert_eq!(result.changed_files(), 0);
    assert!(
        !resolve_agent_paths("cursor", &setup_options)
            .unwrap()
            .mcp_config
            .exists(),
        "an absent config must not be created just to say Leteo is not in it"
    );
}

#[test]
fn codex_keeps_its_home_under_dot_codex_on_every_platform() {
    // Codex reads `~/.codex/config.toml` whatever the operating system.
    // Windows used to be sent to `%APPDATA%\codex`, so `leteo setup codex`
    // wrote a file Codex never read, reported success, and the setup screen
    // read the same file back and called Codex configured. Nothing failed
    // anywhere; it simply did not work.
    let temp = TempDir::new().unwrap();
    let mut setup_options = options(&temp);
    let expected = temp.path().join(".codex").join("config.toml");
    for platform in [Platform::Windows, Platform::MacOs, Platform::Unix] {
        setup_options.platform = Some(platform);
        let codex = resolve_agent_paths("codex", &setup_options).unwrap();
        assert_eq!(codex.mcp_config, expected, "{platform:?}");
        assert_eq!(
            codex.instructions,
            Some(temp.path().join(".codex").join("AGENTS.md")),
            "{platform:?}"
        );
    }
}

#[test]
fn platform_specific_paths_use_the_expected_roots() {
    let temp = TempDir::new().unwrap();
    let mut setup_options = options(&temp);
    setup_options.platform = Some(Platform::Windows);
    let vscode_windows = resolve_agent_paths("vscode-copilot", &setup_options).unwrap();
    assert_eq!(
        vscode_windows.mcp_config,
        temp.path()
            .join("appdata")
            .join("Code")
            .join("User")
            .join("mcp.json")
    );

    setup_options.platform = Some(Platform::MacOs);
    let vscode = resolve_agent_paths("vscode-copilot", &setup_options).unwrap();
    assert_eq!(
        vscode.mcp_config,
        temp.path()
            .join("Library")
            .join("Application Support")
            .join("Code")
            .join("User")
            .join("mcp.json")
    );

    setup_options.platform = Some(Platform::Unix);
    let kilo = resolve_agent_paths("kilocode", &setup_options).unwrap();
    assert_eq!(
        kilo.mcp_config,
        temp.path()
            .join("config")
            .join("kilo")
            .join("opencode.json")
    );
}

/// The Gemini CLI resolves its user settings as `join(homedir(), '.gemini')`
/// with no branch on the operating system, and loads `GEMINI.md` from beside
/// them as context. Writing to `%APPDATA%\gemini` on Windows instead produced a
/// setup that reported success over a file the agent never opens; a real Gemini
/// session asked to list its tools named the servers from `~/.gemini` and none
/// of the ones from `%APPDATA%`. `system.md` was the other half of the same
/// mistake: it is read only when `GEMINI_SYSTEM_MD` is set, and it replaces the
/// whole system prompt when it is.
#[test]
fn the_gemini_cli_is_configured_where_it_actually_reads() {
    let temp = TempDir::new().unwrap();
    let mut setup_options = options(&temp);
    for platform in [Platform::Windows, Platform::MacOs, Platform::Unix] {
        setup_options.platform = Some(platform);
        let gemini = resolve_agent_paths("gemini-cli", &setup_options).unwrap();
        assert_eq!(
            gemini.mcp_config,
            temp.path().join(".gemini").join("settings.json"),
            "{platform:?}"
        );
        assert_eq!(
            gemini.instructions,
            Some(temp.path().join(".gemini").join("GEMINI.md")),
            "{platform:?}"
        );
    }
}

/// The Gemini CLI and Antigravity read one `~/.gemini/GEMINI.md` between them.
/// Taking Leteo out of the first used to splice the block out from under the
/// second, which kept its MCP server and would have run with the tools and none
/// of the instructions for using them — the kind of half-removal nothing
/// reports. The block goes when the last of the two lets go of it.
#[test]
fn taking_leteo_out_of_one_gemini_leaves_the_block_the_other_still_reads() {
    let temp = TempDir::new().unwrap();
    let setup_options = SetupOptions {
        install_instructions: true,
        ..options(&temp)
    };
    let shared = resolve_agent_paths("gemini-cli", &setup_options)
        .unwrap()
        .instructions
        .unwrap();
    assert_eq!(
        resolve_agent_paths("antigravity", &setup_options)
            .unwrap()
            .instructions,
        Some(shared.clone())
    );

    setup("gemini-cli", &setup_options).unwrap();
    setup("antigravity", &setup_options).unwrap();
    let both = fs::read_to_string(&shared).unwrap();
    assert_eq!(
        both.matches(MEMORY_PROTOCOL_BEGIN).count(),
        1,
        "two installs, one block: {both}"
    );

    let result = uninstall("gemini-cli", &setup_options).unwrap();
    let kept = result
        .actions
        .iter()
        .find(|action| action.kind == ActionKind::Instructions)
        .expect("the shared file is reported, not passed over in silence");
    assert_eq!(kept.change, Change::Unchanged);
    assert_eq!(kept.kept_for, Some("antigravity"));
    assert!(
        fs::read_to_string(&shared)
            .unwrap()
            .contains(MEMORY_PROTOCOL_BEGIN),
        "Antigravity still reads this file and still has the server"
    );

    let result = uninstall("antigravity", &setup_options).unwrap();
    let gone = result
        .actions
        .iter()
        .find(|action| action.kind == ActionKind::Instructions)
        .expect("the last one out takes the block");
    assert_eq!(gone.kept_for, None);
    assert!(
        !shared.exists()
            || !fs::read_to_string(&shared)
                .unwrap()
                .contains(MEMORY_PROTOCOL_BEGIN)
    );
}

#[test]
fn invalid_server_block_does_not_overwrite_the_existing_config() {
    let temp = TempDir::new().unwrap();
    let setup_options = options(&temp);
    let paths = resolve_agent_paths("qwen", &setup_options).unwrap();
    let seed = r#"{"theme":"dark","mcpServers":"invalid"}"#;
    write_fixture(&paths.mcp_config, seed);

    let error = setup("qwen", &setup_options).unwrap_err();

    assert!(error.to_string().contains("mcpServers"));
    assert_eq!(fs::read_to_string(paths.mcp_config).unwrap(), seed);
}

#[test]
fn a_block_that_lost_its_end_marker_keeps_what_was_written_after_it() {
    // The end marker went missing — a hand-edit, or a write that did not
    // finish. Whatever follows the orphaned body belongs to the person.
    let damaged = format!("{MEMORY_PROTOCOL_BEGIN}\n\nold body\n\nMy own notes.\n");

    let repaired = upsert_memory_protocol(&damaged);

    assert!(repaired.contains("My own notes."));
    assert_eq!(
        repaired.matches(MEMORY_PROTOCOL_BEGIN).count(),
        1,
        "a second begin marker is what let the next run eat the notes"
    );
    assert_eq!(repaired.matches(MEMORY_PROTOCOL_END).count(), 1);
}

#[test]
fn writing_the_block_twice_over_a_damaged_file_changes_nothing_the_second_time() {
    let damaged = format!("{MEMORY_PROTOCOL_BEGIN}\n\nold body\n\nMy own notes.\n");

    let once = upsert_memory_protocol(&damaged);
    let twice = upsert_memory_protocol(&once);

    assert_eq!(once, twice);
    assert!(twice.contains("My own notes."));
}

#[test]
fn a_file_carrying_two_blocks_comes_back_with_one() {
    // What the old behaviour left behind, and what a file on disk may hold now.
    let doubled = format!(
        "Header.\n\n{MEMORY_PROTOCOL_BEGIN}\n\nfirst\n\n{MEMORY_PROTOCOL_END}\n\nBetween them.\n\n{MEMORY_PROTOCOL_BEGIN}\n\nsecond\n\n{MEMORY_PROTOCOL_END}\n\nFooter.\n"
    );

    let repaired = upsert_memory_protocol(&doubled);

    assert_eq!(repaired.matches(MEMORY_PROTOCOL_BEGIN).count(), 1);
    for line in ["Header.", "Between them.", "Footer."] {
        assert!(repaired.contains(line), "{line} was dropped");
    }
}

#[test]
fn removing_the_protocol_takes_every_block_and_leaves_the_rest() {
    let doubled = format!(
        "Header.\n\n{MEMORY_PROTOCOL_BEGIN}\n\nfirst\n\n{MEMORY_PROTOCOL_END}\n\nBetween them.\n\n{MEMORY_PROTOCOL_BEGIN}\n\nsecond\n\n{MEMORY_PROTOCOL_END}\n"
    );

    let cleaned = remove_memory_protocol(&doubled);

    assert!(!cleaned.contains(MEMORY_PROTOCOL_BEGIN));
    assert!(cleaned.contains("Header."));
    assert!(cleaned.contains("Between them."));
}

#[test]
fn a_file_with_no_block_is_handed_back_untouched() {
    // Setup compares this against what is on disk to decide whether to write.
    // Reformatting a file it has nothing to remove from would report a change
    // that is not one.
    let plain = "# Notes\n\nSomething.\n\n\n";

    assert_eq!(remove_memory_protocol(plain), plain);
}

/// Writes a plugin bundle's hooks manifest where the agent would find one.
fn install_plugin_bundle(temp: &TempDir, version: &str) -> PathBuf {
    let manifest = temp
        .path()
        .join(".claude/plugins/cache/leteo/leteo")
        .join(version)
        .join("hooks/hooks.json");
    write_fixture(
        &manifest,
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"leteo hook session-start"}]}]}}"#,
    );
    manifest
}

#[test]
fn setup_refuses_to_install_hooks_the_plugin_already_registers() {
    // The bundle and this command write to two different files and the agent
    // reads both, so two registrations fire every event twice — the session
    // opening reported twice, and every prompt stored twice. It looked like a
    // repeated line on screen and it was duplicated data.
    let temp = TempDir::new().unwrap();
    install_plugin_bundle(&temp, "0.1.0");
    let setup_options = SetupOptions {
        install_hooks: true,
        ..options(&temp)
    };

    let error = setup("claude-code", &setup_options)
        .unwrap_err()
        .to_string();

    assert!(error.contains("already registers"), "{error}");
    assert!(
        error.contains("twice"),
        "the message has to say what goes wrong, not just that it refused: {error}"
    );
    let paths = resolve_agent_paths("claude-code", &setup_options).unwrap();
    assert!(
        !paths.hooks.unwrap().exists(),
        "and nothing may have been written on the way to refusing"
    );
}

#[test]
fn a_bundle_of_any_version_is_the_one_that_would_fire() {
    // The path carries whatever version was installed, so the check walks the
    // cache rather than spelling one out.
    let temp = TempDir::new().unwrap();
    install_plugin_bundle(&temp, "9.9.9-rc1");

    assert!(
        setup(
            "claude-code",
            &SetupOptions {
                install_hooks: true,
                ..options(&temp)
            }
        )
        .is_err()
    );
}

#[test]
fn setup_still_installs_hooks_when_no_plugin_is_present() {
    let temp = TempDir::new().unwrap();
    let setup_options = SetupOptions {
        install_hooks: true,
        ..options(&temp)
    };

    let result = setup("claude-code", &setup_options).unwrap();

    assert!(
        result
            .actions
            .iter()
            .any(|action| action.kind == ActionKind::Hooks),
        "the only reason to refuse is a bundle that already did it"
    );
}

#[test]
fn claude_paths_follow_the_config_directory_rather_than_assuming_the_profile() {
    // `CLAUDE_CONFIG_DIR` moves the whole directory — settings, hooks and the
    // plugin cache — and writing to `~/.claude` anyway is the failure the Codex
    // adapter already carries a note about: setup reports success and the agent
    // reads a different file.
    //
    // The variable is process-wide, so this reads the resolution rule rather
    // than mutating it, the way `paths.rs` does for HOME.
    let temp = TempDir::new().unwrap();
    let relocated = temp.path().join("elsewhere").join("claude");

    let moved = SetupEnvironment {
        claude_config: Some(relocated.clone()),
        ..SetupEnvironment::resolve(&options(&temp)).unwrap()
    };
    assert_eq!(moved.claude_config_dir(), relocated);

    let untouched = SetupEnvironment {
        claude_config: None,
        ..SetupEnvironment::resolve(&options(&temp)).unwrap()
    };
    assert_eq!(untouched.claude_config_dir(), temp.path().join(".claude"));

    // An empty or relative value says nothing, and taking it literally would
    // send every Claude path to the filesystem root or to the process's cwd.
    assert_eq!(resolve_optional_root(None, Some(PathBuf::from(""))), None);
    assert_eq!(
        resolve_optional_root(None, Some(PathBuf::from("relative/claude"))),
        None
    );
}

#[test]
fn hooks_and_the_plugin_guard_read_the_same_relocated_directory() {
    // Both have to move together. A guard looking in the old place while the
    // write goes to the new one would install the duplicate it exists to stop.
    let temp = TempDir::new().unwrap();
    let relocated = temp.path().join("elsewhere").join("claude");
    let manifest = relocated.join("plugins/cache/leteo/leteo/0.1.0/hooks/hooks.json");
    write_fixture(
        &manifest,
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"leteo hook session-start"}]}]}}"#,
    );
    let environment = SetupEnvironment {
        claude_config: Some(relocated.clone()),
        ..SetupEnvironment::resolve(&options(&temp)).unwrap()
    };

    let claude_code = find_adapter("claude-code").unwrap();
    assert_eq!(
        installed_plugin_hooks(&environment, claude_code),
        Some(manifest)
    );
    // Codex keeps its bundles under its own home, so a Claude Code plugin is
    // not a reason to withhold hooks from it.
    assert_eq!(
        installed_plugin_hooks(&environment, find_adapter("codex").unwrap()),
        None
    );
    assert_eq!(
        resolve_paths(find_adapter("claude-code").unwrap(), &environment).hooks,
        Some(relocated.join("settings.json"))
    );
}

/// Codex gets hooks, in the file it actually reads, beside the server.
///
/// Codex takes hooks as `[[hooks.<Event>]]` in `config.toml` — the same file
/// the MCP server goes in, not a settings file of its own. That shape is what
/// makes this worth a test: two renders write one file in sequence, and the
/// second has to build on the first rather than replace it.
#[test]
fn codex_hooks_land_in_config_toml_beside_the_server() {
    let temp = TempDir::new().unwrap();
    let config = temp.path().join(".codex").join("config.toml");
    // Something of the user's, including a hook that is none of Leteo's
    // business, to be sure neither is lost.
    write_fixture(
        &config,
        "model = \"gpt-5\"\n\n[[hooks.SessionStart]]\n\n[[hooks.SessionStart.hooks]]\ntype = \"command\"\ncommand = \"echo mine\"\n",
    );

    let result = setup(
        "codex",
        &SetupOptions {
            install_hooks: true,
            ..options(&temp)
        },
    )
    .unwrap();
    assert!(result.changed_files() > 0);

    let written = fs::read_to_string(&config).unwrap();
    assert!(written.contains("[mcp_servers.leteo]"), "{written}");
    assert!(written.contains("[[hooks.SessionStart]]"), "{written}");
    assert!(written.contains("hook session-start"), "{written}");
    assert!(written.contains("hook user-prompt-submit"), "{written}");
    assert!(written.contains("hook session-stop"), "{written}");
    assert!(
        written.contains("matcher = \"startup|clear\""),
        "the compaction hook has to be told apart from the opening one: {written}"
    );
    assert!(written.contains("model = \"gpt-5\""), "{written}");
    assert!(
        written.contains("echo mine"),
        "somebody else's hook is not Leteo's to remove: {written}"
    );

    // Run again: an upsert, not an append. Registering twice is what makes
    // every event fire twice, which is the whole reason the plugin guard exists.
    setup(
        "codex",
        &SetupOptions {
            install_hooks: true,
            ..options(&temp)
        },
    )
    .unwrap();
    let again = fs::read_to_string(&config).unwrap();
    assert_eq!(
        again.matches("hook session-start").count(),
        1,
        "a second run duplicated the hooks: {again}"
    );
    assert_eq!(again.matches("[mcp_servers.leteo]").count(), 1, "{again}");
    assert_eq!(again.matches("echo mine").count(), 1, "{again}");

    // Taking Leteo out has to take both, since both are in this file. A hook
    // left behind goes on invoking a binary that is no longer configured.
    uninstall("codex", &options(&temp)).unwrap();
    let after = fs::read_to_string(&config).unwrap();
    assert!(!after.contains("[mcp_servers.leteo]"), "{after}");
    assert!(!after.contains("hook session-start"), "{after}");
    assert!(!after.contains("hook user-prompt-submit"), "{after}");
    assert!(after.contains("model = \"gpt-5\""), "{after}");
    assert!(
        after.contains("echo mine"),
        "and somebody else's hook still is not Leteo's to remove: {after}"
    );
}

/// The bundles declare the same hooks the setup command writes.
///
/// Two ways to install Leteo, two places the events and their timeouts are
/// spelled out, and nothing held them together. They had already drifted where
/// it showed: both bundles asked five seconds for `SessionEnd`, which Codex
/// caps at three, so every session opened with "clamping SessionEnd hook
/// timeout to 3s". A warning on every session is the cheap version of this
/// failure — the expensive one is an event a bundle registers and setup does
/// not, firing for half the users.
///
/// # Why the matcher is compared and not only the timeout
///
/// This compared `(event, subcommand) -> timeout` and let the matcher through,
/// which left the one field that carries meaning on its own unwatched. Both
/// `session-start` and `post-compaction` sit on `SessionStart`; the matcher is
/// the whole of what tells them apart. Editing the bundle's `compact` to
/// `startup` was measured against this test before the matcher was added to
/// it, and the test stayed green — a Leteo that reopens the session context at
/// startup twice and never recovers after a compaction, which is the one
/// moment the bundle exists for, with nothing anywhere saying so.
#[test]
fn the_plugin_bundles_register_the_hooks_the_binary_writes() {
    let expected: BTreeMap<(String, String), (Option<String>, u64)> = HOOK_EVENTS
        .iter()
        .map(|registration| {
            (
                (registration.event.to_owned(), registration.slug.to_owned()),
                (
                    registration.matcher.map(str::to_owned),
                    registration.timeout_seconds,
                ),
            )
        })
        .collect();

    for bundle in [
        "plugin/claude-code/hooks/hooks.json",
        "plugin/codex/hooks/hooks.json",
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(bundle);
        let manifest: Value = serde_json::from_slice(&fs::read(&path).unwrap())
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));

        let mut found = BTreeMap::new();
        for (event, groups) in manifest["hooks"].as_object().expect("an events object") {
            for group in groups.as_array().expect("a list of matcher groups") {
                // Absent and empty mean the same thing to both agents — every
                // invocation of the event — and `HOOK_EVENTS` spells that
                // `None`. Left as `Some("")`, a bundle that writes the key out
                // with nothing in it would fail this for no reason.
                let matcher = group["matcher"]
                    .as_str()
                    .filter(|matcher| !matcher.is_empty())
                    .map(str::to_owned);
                for hook in group["hooks"].as_array().expect("a list of handlers") {
                    let command = hook["command"].as_str().expect("a command");
                    let slug = command
                        .rsplit_once("hook ")
                        .map(|(_, slug)| slug.to_owned())
                        .unwrap_or_else(|| panic!("{bundle} runs something else: {command}"));
                    let timeout = hook["timeout"].as_u64().expect("a timeout");
                    found.insert((event.clone(), slug), (matcher.clone(), timeout));
                }
            }
        }

        assert_eq!(
            found, expected,
            "{bundle} and HOOK_EVENTS disagree about which hooks run, on what, for how long"
        );
    }
}

/// ZCode gets its hooks registered inside the same JSON document as the
/// server, and the client keeps everything else it owns in that document too.
///
/// Providers, plugin state and MCP servers all live in `~/.zcode/cli/config.json`,
/// alongside hooks nested two keys deep (`hooks.events.<Event>`) behind an
/// `enabled` switch that starts off. So this has more to lose than any other
/// agent's file: a render that replaced rather than spliced would drop
/// somebody's provider configuration out from under them.
///
/// Three registrations land, not five: this client supports neither
/// `SubagentStop` nor `SessionEnd`, and `session-stop` deliberately does not
/// move onto `Stop`, which fires at the end of every reply — see the note on
/// `ZCODE_HOOK_REGISTRATIONS`.
#[test]
fn zcode_hooks_land_in_config_json_beside_the_server() {
    const OFF: &str = r#"{
  "provider": {"builtin:bigmodel": {"enabled": false}},
  "mcp": {
    "servers": {"other": {"type": "stdio", "command": "uvx", "args": ["other-mcp"]}}
  },
  "hooks": {
    "enabled": false,
    "events": {
      "PostToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "warden watch"}]}]
    }
  }
}"#;

    let temp = TempDir::new().unwrap();
    let config = temp.path().join(".zcode").join("cli").join("config.json");
    // Something of the person's on every axis this touches: another MCP
    // server, a hook that is none of Leteo's business, a scalar setting, and
    // the runner deliberately left off before Leteo ever arrived.
    write_fixture(&config, OFF);

    let setup_options = SetupOptions {
        install_hooks: true,
        ..options(&temp)
    };
    // Switched off at the wall: refusing beats writing registrations into a
    // block the client will never read, which is success shaped exactly like
    // the failure that follows silently otherwise.
    let error = setup("zcode", &setup_options).unwrap_err().to_string();
    assert!(error.contains("hooks.enabled"), "{error}");
    // And nothing went in ahead of the refusal — not even a server entry whose
    // hooks could never run.
    assert_eq!(fs::read_to_string(&config).unwrap(), OFF);

    // The person turns their own runner on, and now Leteo may join it.
    // Anchored on the trailing comma, which only the hooks switch has — the
    // client's own off-switch for its provider reads `"enabled": false}`
    // without one, and stays as it was.
    write_fixture(
        &config,
        &OFF.replace("\"enabled\": false,", "\"enabled\": true,"),
    );
    setup("zcode", &setup_options).unwrap();

    let written = read_json(&config);
    assert_eq!(
        written["mcp"]["servers"]["other"]["command"], "uvx",
        "{written}"
    );
    let leteo = &written["mcp"]["servers"]["leteo"];
    assert_eq!(leteo["type"], "stdio", "{written}");
    assert_eq!(
        leteo["args"],
        serde_json::json!(["mcp", "--tools=agent"]),
        "{written}"
    );

    let hooks = &written["hooks"];
    assert_eq!(hooks["enabled"], true, "{written}");
    let events = hooks["events"].as_object().unwrap();
    assert!(
        events.keys().all(|event| {
            matches!(
                event.as_str(),
                "SessionStart" | "UserPromptSubmit" | "PostToolUse"
            )
        }),
        "no event is written that ZCode does not support: {events:?}"
    );
    assert!(events.get("SubagentStop").is_none(), "{written}");
    assert!(events.get("SessionEnd").is_none(), "{written}");
    let starters = events["SessionStart"].as_array().unwrap();
    let matchers: Vec<&str> = starters
        .iter()
        .map(|entry| entry["matcher"].as_str().expect("a matcher"))
        .collect();
    assert_eq!(
        matchers,
        ["startup|clear", "compact"],
        "the opening hook must be told apart from the compaction one: {starters:?}"
    );
    assert_eq!(events["UserPromptSubmit"][0]["hooks"][0]["timeout"], 5);

    // A second run is an upsert, not a duplicate registration: every event
    // firing twice stores each prompt twice.
    setup("zcode", &setup_options).unwrap();
    let again = fs::read_to_string(&config).unwrap();
    assert_eq!(
        again.matches("hook session-start").count(),
        1,
        "a second run duplicated the hooks: {again}"
    );
    // The one server named leteo, twice written once — and "other" untouched.
    assert_eq!(again.matches("other-mcp").count(), 1, "{again}");
    assert_eq!(again.matches("warden watch").count(), 1, "{again}");

    // Taking Leteo out takes the hooks with it, and leaves every other thing
    // in the document — including the runner's switch itself, which was not
    // ours to turn either way.
    uninstall("zcode", &options(&temp)).unwrap();
    let after = fs::read_to_string(&config).unwrap();
    let parsed = read_json(&config);
    assert!(
        parsed["mcp"]["servers"]
            .as_object()
            .unwrap()
            .get("leteo")
            .is_none(),
        "{after}"
    );
    assert!(parsed["hooks"]["enabled"] == true, "{after}");
    assert!(parsed["hooks"]["events"].get("SessionStart").is_none());
    assert!(parsed["hooks"]["events"].get("UserPromptSubmit").is_none());
    assert!(
        parsed["hooks"]["events"]
            .get("PostToolUse")
            .is_some_and(|entries| entries
                .as_array()
                .is_some_and(|entries| !entries.is_empty())),
        "somebody else's hook stays registered: {after}"
    );
    assert_eq!(parsed["mcp"]["servers"]["other"]["command"], "uvx");
}

/// And an empty hook block goes entirely when uninstall empties it: a key
/// saying nothing about hooks says nothing by being there.
#[test]
fn zcode_uninstall_removes_an_events_map_it_emptied() {
    let temp = TempDir::new().unwrap();

    setup(
        "zcode",
        &SetupOptions {
            install_hooks: true,
            ..options(&temp)
        },
    )
    .unwrap();
    let config = temp.path().join(".zcode").join("cli").join("config.json");
    let installed = read_json(&config);
    assert_eq!(installed["hooks"]["enabled"], true);

    uninstall("zcode", &options(&temp)).unwrap();
    let after = read_json(&config);
    // Every registration removed was Leteo's, so the events map went with
    // them — emptied keys say nothing and do not stay (the same judgement the
    // flat settings-file uninstall makes one level up). What remains is the
    // runner switch alone: inert with nothing under it to run, and not ours
    // to turn back off once somebody else may have come to rely on.
    let hooks = after.get("hooks").and_then(serde_json::Value::as_object);
    assert_eq!(hooks.map(|hooks| hooks.len()), Some(1), "{after}");
    assert_eq!(after["hooks"]["enabled"], true, "{after}");

    // One instruction file goes beside all that, and uninstall takes its
    // block without leaving the file: ~/.zcode/AGENTS.md did not exist before
    // and installing created it.
    assert!(
        !temp.path().join(".zcode").join("AGENTS.md").exists(),
        "an instruction file nothing else reads is not left behind"
    );
}

/// A plugin bundle, in the cache the given agent reads it from.
fn install_bundle(home: &Path, agent_dir: &str) {
    let hooks = home
        .join(agent_dir)
        .join("plugins")
        .join("cache")
        .join("leteo")
        .join("leteo")
        .join("0.1.0")
        .join("hooks");
    write_fixture(
        &hooks.join("hooks.json"),
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"leteo hook session-start"}]}]}}"#,
    );
}

#[test]
fn doctor_can_see_hooks_that_are_installed_and_never_fire() {
    // The README has always sent people to `leteo doctor` when a hook goes
    // quiet, and doctor could only answer for SQLite — it reported a perfectly
    // healthy store to somebody whose hooks had never run once.
    let temp = TempDir::new().unwrap();
    let options = options(&temp);

    // Nothing installed anywhere: three agents take hooks, and none has any.
    let health = hook_health(&options);
    assert_eq!(
        health.iter().map(|agent| agent.agent).collect::<Vec<_>>(),
        ["claude-code", "zcode", "codex"],
        "only the agents that can take hooks are reported"
    );
    assert!(
        health.iter().all(|agent| agent.configured.is_none()
            && agent.bundled.is_none()
            && agent.issue.is_none()),
        "{health:?}"
    );

    // Hooks written into Codex's config, and Codex never asked to trust them.
    // They are installed, they are correct, and not one of them runs.
    setup(
        "codex",
        &SetupOptions {
            install_hooks: true,
            ..options.clone()
        },
    )
    .unwrap();
    let codex = |health: &[HookHealth]| {
        health
            .iter()
            .find(|agent| agent.agent == "codex")
            .expect("Codex takes hooks")
            .clone()
    };
    let untrusted = codex(&hook_health(&options));
    assert!(untrusted.configured.is_some(), "{untrusted:?}");
    let issue = untrusted.issue.expect("untrusted hooks are an issue");
    assert!(issue.contains("trust"), "{issue}");

    // Trust granted, and there is nothing left to say.
    let config = temp.path().join(".codex").join("config.toml");
    let trusted = fs::read_to_string(&config).unwrap()
        + "\n[hooks.state.\"whatever\"]\ntrusted_hash = \"sha256:abc\"\n";
    fs::write(&config, trusted).unwrap();
    assert!(codex(&hook_health(&options)).issue.is_none());

    // And then the plugin bundle arrives on top of the hooks setup wrote.
    // Setup refuses to add the second copy; nothing stops this order, and
    // every event now fires twice.
    install_bundle(temp.path(), ".codex");
    let doubled = codex(&hook_health(&options));
    assert!(doubled.configured.is_some() && doubled.bundled.is_some());
    let issue = doubled.issue.expect("two registrations are an issue");
    assert!(issue.contains("twice"), "{issue}");

    // A bundle on its own is the ordinary supported install, not a fault —
    // and the trust question is not asked of an agent that never asks it.
    install_bundle(temp.path(), ".claude");
    let claude = hook_health(&options)
        .into_iter()
        .find(|agent| agent.agent == "claude-code")
        .expect("Claude Code takes hooks");
    assert!(claude.bundled.is_some() && claude.configured.is_none());
    assert!(claude.issue.is_none(), "{claude:?}");
}

/// Somebody's own document comes back with the line endings it had.
///
/// Both editors of an instruction file fold `\r\n` to `\n` so their matching
/// has one shape to look for, and both used to write the folded text back. On
/// Windows that turns installing or removing one block into a rewrite of every
/// line of a file the person wrote — a diff where the change is buried in noise
/// nobody asked for, and on a repository with `core.autocrlf` unset, a commit
/// that touches the whole file.
///
/// The refusal in `remove_memory_protocol` to report an untouched file as
/// changed was the same concern, caught for one case.
#[test]
fn an_instruction_file_keeps_the_line_endings_it_arrived_with() {
    let theirs = "# My notes\r\n\r\nSomething I wrote.\r\n";

    let installed = upsert_memory_protocol(theirs);
    assert!(
        installed.contains(MEMORY_PROTOCOL_BEGIN),
        "the block still has to land"
    );
    assert!(
        installed.starts_with("# My notes\r\n"),
        "their lines have to survive as they were: {:?}",
        &installed[..40.min(installed.len())]
    );
    assert!(
        !installed.replace("\r\n", "").contains('\n'),
        "no line may be left bare when the file was CRLF"
    );

    // And out again, back to exactly what they started with.
    let removed = remove_memory_protocol(&installed);
    assert_eq!(removed, theirs);

    // A file that was LF stays LF — the conversion runs one way only.
    let unix = "# My notes\n\nSomething I wrote.\n";
    let installed = upsert_memory_protocol(unix);
    assert!(!installed.contains('\r'), "an LF file must not gain CRLF");
    assert_eq!(remove_memory_protocol(&installed), unix);

    // A file with no block is handed back untouched rather than reformatted.
    assert_eq!(remove_memory_protocol(theirs), theirs);
}

/// A configuration file belongs to whoever wrote it, keys and order included.
///
/// `serde_json`'s object is a sorted map unless `preserve_order` is on, so
/// parsing a config and writing it back reordered every key alphabetically.
/// The file this lands in is not Leteo's: a real `.claude.json` holds 82
/// top-level keys over 82 KB — startup counts, tips history, per-project state
/// — and adding one MCP server rewrote 240 lines of it. Semantically identical,
/// and a diff in which the actual change cannot be found.
///
/// Measured after: 4 lines, which is the change itself.
#[test]
fn adding_a_server_leaves_the_rest_of_the_configuration_where_it_was() {
    // Deliberately not alphabetical, and `$schema` first the way editors
    // expect — sorting would move it below everything.
    let theirs = r#"{
  "$schema": "https://example.com/schema.json",
  "numStartups": 41,
  "autoUpdates": true,
  "mcpServers": {
    "zulu": {
      "command": "zulu"
    }
  },
  "alpha": "last on purpose"
}
"#;
    let path = std::path::Path::new("mcp.json");
    let rendered = render::render_json_config(
        path,
        Some(theirs.as_bytes()),
        McpFormat::Servers,
        std::path::Path::new("/usr/bin/leteo"),
        "agent",
    )
    .unwrap();

    let order: Vec<&str> = rendered
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|line| line.split_once('"').map(|(key, _)| key))
        .collect();
    let theirs_order = ["$schema", "numStartups", "autoUpdates", "mcpServers"];
    for pair in theirs_order.windows(2) {
        let first = order.iter().position(|key| *key == pair[0]);
        let second = order.iter().position(|key| *key == pair[1]);
        assert!(
            first < second,
            "{:?} moved before {:?}: {order:?}",
            pair[1],
            pair[0]
        );
    }
    assert!(
        order.iter().position(|key| *key == "alpha") > order.iter().position(|key| *key == "zulu"),
        "sorting would have pulled alpha above zulu: {order:?}"
    );
    assert!(
        rendered.contains("\"leteo\""),
        "the server still has to land"
    );
}

/// The same for Codex, whose server, hooks and config share one TOML file.
///
/// Five places fold `\r\n` to `\n` to do their matching. Two were paired with
/// the ending going back; three were not, and all three write the file a Codex
/// user keeps their own settings and API keys in.
#[test]
fn the_codex_config_keeps_the_line_endings_it_arrived_with() {
    let theirs =
        "[model]\r\nname = \"gpt-5\"\r\n\r\n# a comment they wrote\r\n[other]\r\nkey = 1\r\n";
    let leteo = std::path::Path::new("/usr/bin/leteo");

    for (label, rendered) in [
        (
            "server",
            render_codex_config(Some(theirs.as_bytes()), "/usr/bin/leteo", "agent").unwrap(),
        ),
        (
            "hooks",
            render::render_codex_hooks(Some(theirs.as_bytes()), HOOK_EVENTS, leteo).unwrap(),
        ),
    ] {
        assert!(
            !rendered.replace("\r\n", "").contains('\n'),
            "{label}: no line may be left bare when the file was CRLF"
        );
        assert!(
            rendered.contains("# a comment they wrote"),
            "{label}: their comment has to survive"
        );
    }

    // And out again. Uninstalling reads the same file and writes it back.
    let installed =
        render_codex_config(Some(theirs.as_bytes()), "/usr/bin/leteo", "agent").unwrap();
    let removed = remove_codex_server(installed.as_bytes()).unwrap();
    assert!(
        !removed.replace("\r\n", "").contains('\n'),
        "removal must not flatten the file either"
    );
    assert_eq!(removed, theirs.trim_end().to_owned() + "\r\n");

    // An LF file stays LF: the conversion runs one way only.
    let unix = theirs.replace("\r\n", "\n");
    let installed = render_codex_config(Some(unix.as_bytes()), "/usr/bin/leteo", "agent").unwrap();
    assert!(!installed.contains('\r'), "an LF file must not gain CRLF");
}

#[test]
fn detected_indent_reads_four_spaces() {
    let four = "{\n    \"a\": {\n        \"b\": 1\n    }\n}\n";
    assert_eq!(super::detected_indent(four), "    ");
    assert_eq!(super::detected_indent("{\n  \"a\": 1\n}\n"), "  ");
    assert_eq!(super::detected_indent("{\"a\":1}"), "  ");
}

#[test]
fn a_config_comes_back_indented_the_way_it_arrived() {
    // Adding one server to an eighty-line four-space config rewrote a hundred
    // and sixty-seven lines of it, because `to_string_pretty` always uses two.
    let existing = "{\n    \"mcpServers\": {\n        \"mine\": {\n            \"command\": \"x\"\n        }\n    }\n}\n";
    // And the same file as Windows keeps it, which is where this was found: a
    // config written on Windows has CRLF, and every check here has to hold for
    // it too or the fix only works on the machine it was written on.
    let windows = existing.replace('\n', "\r\n");
    for existing in [existing, windows.as_str()] {
        let rendered = render_json_config(
            Path::new("/tmp/.claude.json"),
            Some(existing.as_bytes()),
            McpFormat::McpServers,
            Path::new("/usr/bin/leteo"),
            "agent",
        )
        .unwrap();
        let body = rendered.replace("\r\n", "\n");
        assert!(
            body.contains("\n    \"mcpServers\""),
            "four spaces in, four spaces out:\n{rendered}"
        );
        assert!(
            !body.contains("\n  \"mcpServers\""),
            "reindented to two:\n{rendered}"
        );
        // The endings it arrived with, for the same reason the instruction
        // files keep theirs: a config that comes back LF has every one of its
        // lines changed, and the one Leteo added is lost in the noise.
        assert_eq!(
            rendered.contains("\r\n"),
            existing.contains("\r\n"),
            "line endings were not the ones this file had:\n{rendered:?}"
        );
    }
}

/// Taking Leteo's hooks out of a Codex config leaves everybody else's alone.
///
/// The file is shared: Codex keeps every tool's hooks in one `config.toml`, and
/// this runs on install as well as on uninstall. Leteo has already rewritten
/// somebody else's document twice — a plugin block that took 240 lines of a
/// config with it, and an installer that deleted the directory it was installed
/// into — so the rule is worth a fixture that fights back.
#[test]
fn removing_leteo_hooks_keeps_every_other_tool_in_the_file() {
    // The shape the renderer writes: the command is one string, the executable
    // quoted inside it. A fixture that spells it as a TOML array asserts
    // nothing, because that is not what is being looked for.
    let file = concat!(
        "[[hooks.SessionStart]]\n",
        "\n[[hooks.SessionStart.hooks]]\n",
        "type = \"command\"\n",
        "command = \"warden --dialect claude-code\"\n",
        "timeout = 10\n",
        "\n[[hooks.SessionStart]]\n",
        "\n[[hooks.SessionStart.hooks]]\n",
        "type = \"command\"\n",
        "command = \"\\\"C:/Users/alguien/leteo.exe\\\" hook session-start\"\n",
        "timeout = 10\n",
        "\n[[hooks.UserPromptSubmit]]\n",
        "\n[[hooks.UserPromptSubmit.hooks]]\n",
        "type = \"command\"\n",
        "command = \"otra-herramienta --nota sustituye-a-leteo\"\n",
        "\n[[hooks.SessionEnd]]\n",
        "\n[[hooks.SessionEnd.hooks]]\n",
        "type = \"command\"\n",
        "command = \"leteo-extra hook session-stop\"\n",
        "\n[otra.seccion]\n",
        "clave = \"valor\"\n",
    );
    let lines: Vec<&str> = file.lines().collect();
    let kept = crate::setup::render::without_leteo_codex_hooks(&lines).join("\n");

    assert!(
        !kept.contains("leteo.exe"),
        "Leteo's own group is what this removes:\n{kept}"
    );
    assert!(
        kept.contains("warden"),
        "another tool's hook for the same event stays:\n{kept}"
    );
    assert!(
        kept.contains("otra-herramienta"),
        "and one that merely mentions leteo in its arguments is not Leteo's:\n{kept}"
    );
    assert!(
        kept.contains("leteo-extra"),
        "nor is a different binary whose name starts the same:\n{kept}"
    );
    assert!(
        kept.contains("[otra.seccion]"),
        "and nothing outside the hook groups is touched:\n{kept}"
    );
}

/// Every hook the installer writes names a subcommand this binary accepts.
///
/// The five event names are spelled in three places: `HOOK_EVENTS` here, the
/// plugin bundles, and `HookEventArgument`, which is what `leteo hook <event>`
/// will actually parse. A test already holds the first two together. Nothing
/// held either of them to the third.
///
/// What that costs if it ever drifts is quiet. Renaming a variant changes what
/// clap accepts; the installer goes on writing the old word into everybody's
/// settings; and every hook fails with "unrecognized subcommand" — which
/// nobody sees, because a hook that fails is a hook that says nothing, by
/// design, so that a broken Leteo never blocks a prompt. Memory would simply
/// stop being recorded.
#[test]
fn every_hook_the_installer_writes_is_a_subcommand_the_binary_parses() {
    use clap::ValueEnum;

    let accepted: Vec<String> = crate::cli::HookEventArgument::value_variants()
        .iter()
        .filter_map(|variant| variant.to_possible_value())
        .map(|value| value.get_name().to_owned())
        .collect();

    for registration in HOOK_EVENTS {
        let (slug, event) = (registration.slug, registration.event);
        assert!(
            accepted.iter().any(|name| name == slug),
            "the installer writes `leteo hook {slug}` for {event}, and the binary \
             only parses {accepted:?}"
        );
    }

    // And the other direction, because an event this binary can run that the
    // installer never registers is a feature nobody's agent ever reaches.
    for name in &accepted {
        assert!(
            HOOK_EVENTS
                .iter()
                .any(|registration| registration.slug == name),
            "`leteo hook {name}` exists and no agent is ever configured to call it"
        );
    }
}

/// The store gives up waiting before the agent gives up on the hook.
///
/// Two numbers written in two files: how long the installer tells an agent to
/// wait, here, and how long the store waits for another writer, on
/// `HookEvent`. They have to agree, and the second has to be the smaller —
/// otherwise a hook that meets a busy store is killed mid-wait, which tells
/// nobody anything, instead of answering with a warning.
#[test]
fn a_hook_stops_waiting_before_the_agent_stops_waiting_for_it() {
    use crate::hooks::HookEvent;

    let events = [
        ("session-start", HookEvent::SessionStart),
        ("post-compaction", HookEvent::PostCompaction),
        ("user-prompt-submit", HookEvent::UserPromptSubmit),
        ("subagent-stop", HookEvent::SubagentStop),
        ("session-stop", HookEvent::SessionStop),
    ];
    for (slug, event) in events {
        let registered = HOOK_EVENTS
            .iter()
            .find(|registration| registration.slug == slug)
            .map(|registration| registration.timeout_seconds)
            .unwrap_or_else(|| panic!("{slug} is registered"));
        assert_eq!(
            event.agent_timeout_seconds(),
            registered,
            "{slug}: the event and the installer disagree about how long an agent waits"
        );
        assert!(
            event.store_wait() < std::time::Duration::from_secs(registered),
            "{slug}: the store waits {:?} and the agent kills it at {registered}s",
            event.store_wait()
        );
    }
}

/// Uninstalling leaves nothing of Leteo's behind, on every agent.
///
/// `leteo uninstall` says it removes Leteo from this machine entirely, and on
/// three of the thirteen it did not. Nine agents keep their instructions in a file
/// that was already theirs — `CLAUDE.md`, `AGENTS.md` — and uninstalling takes
/// Leteo's block out and leaves the rest, which is right. The other three get a
/// file Leteo invented and named after itself: `leteo-memory-protocol.md` for
/// Cursor, `leteo.md` for Kiro, `leteo.instructions.md` for VS Code Copilot.
/// Taking the block out of one of those left an empty file with Leteo's name on
/// it — and for Copilot, not even empty: the `applyTo` front matter stayed,
/// which is a registered instruction file that applies to every source file and
/// says nothing.
///
/// Driven over the whole registry rather than the three, because the next agent
/// with a file of its own will be added by somebody who has not read this.
#[test]
fn uninstalling_leaves_no_file_of_leteos_behind() {
    for adapter in super::supported_agents() {
        let temp = TempDir::new().unwrap();
        let setup_options = options(&temp);
        setup(
            adapter.slug,
            &SetupOptions {
                // Only what the agent takes, in both cases: asking one for
                // something it has no file for is refused, and refusing is the
                // right answer.
                install_instructions: adapter.instruction_path.is_some(),
                install_hooks: adapter.hooks_path.is_some(),
                ..setup_options.clone()
            },
        )
        .unwrap_or_else(|error| panic!("install {}: {error}", adapter.slug));

        let planted = files_naming_leteo(temp.path());
        assert!(
            !planted.is_empty(),
            "{} installed nothing this could check",
            adapter.slug
        );

        uninstall(adapter.slug, &setup_options)
            .unwrap_or_else(|error| panic!("uninstall {}: {error}", adapter.slug));

        let left = files_naming_leteo(temp.path());
        assert!(
            left.is_empty(),
            "{} leaves {left:?} behind, and `uninstall` says it removes Leteo entirely",
            adapter.slug
        );
    }
}

/// And it takes nothing that was not Leteo's, which is the other half.
///
/// Separate from the guard above because it is a different sentence, and
/// because the two disagree about one file on purpose: an instruction file
/// Leteo named after itself, which somebody has since written their own line
/// into, stays. "Delete the file when it is ours" is one edit away from
/// deleting a document that was only partly ours, and a mutation showed that
/// edit passing.
#[test]
fn uninstalling_keeps_every_line_that_was_not_leteos() {
    const THEIRS: &str = "A line somebody wrote themselves and asked nobody about.";

    for adapter in super::supported_agents() {
        let Some(instruction_path) = adapter.instruction_path else {
            continue;
        };
        let temp = TempDir::new().unwrap();
        let setup_options = options(&temp);
        let environment = SetupEnvironment::resolve(&setup_options).unwrap();
        let theirs = instruction_path(&environment, &(adapter.config_path)(&environment));
        std::fs::create_dir_all(theirs.parent().unwrap()).unwrap();
        std::fs::write(
            &theirs,
            format!(
                "{THEIRS}
"
            ),
        )
        .unwrap();

        setup(
            adapter.slug,
            &SetupOptions {
                install_instructions: true,
                install_hooks: adapter.hooks_path.is_some(),
                ..setup_options.clone()
            },
        )
        .unwrap_or_else(|error| panic!("install {}: {error}", adapter.slug));
        uninstall(adapter.slug, &setup_options)
            .unwrap_or_else(|error| panic!("uninstall {}: {error}", adapter.slug));

        let kept = std::fs::read_to_string(&theirs).unwrap_or_default();
        assert!(
            kept.contains(THEIRS),
            "{} took away a line nobody asked it to touch, in {}",
            adapter.slug,
            theirs.display()
        );

        // And the emptier case, which is the one the ownership flag exists for.
        // A `CLAUDE.md` that was there and empty before Leteo arrived is still
        // somebody's file: stripping the block leaves nothing, and "nothing
        // left" must not be read as "Leteo made this". Only an agent whose
        // instruction file Leteo invented gets that reading.
        // Whether the file is Leteo's is stated on the adapter and is also
        // visible in the name the adapter's own path function picks. Two facts
        // written separately, so they can be held against each other — reading
        // the flag to decide what to check would let a wrong flag skip its own
        // guard, which is what a mutation showed happening.
        let invented = theirs
            .file_name()
            .map(|name| name.to_string_lossy().to_lowercase().contains("leteo"))
            .unwrap_or(false);
        assert_eq!(
            adapter.owns_instruction_file,
            invented,
            "{} claims {} is Leteo's, and the name says otherwise",
            adapter.slug,
            theirs.display()
        );

        if !invented {
            let temp = TempDir::new().unwrap();
            let setup_options = options(&temp);
            let environment = SetupEnvironment::resolve(&setup_options).unwrap();
            let empty = instruction_path(&environment, &(adapter.config_path)(&environment));
            std::fs::create_dir_all(empty.parent().unwrap()).unwrap();
            std::fs::write(&empty, "").unwrap();
            setup(
                adapter.slug,
                &SetupOptions {
                    install_instructions: true,
                    ..setup_options.clone()
                },
            )
            .unwrap();
            uninstall(adapter.slug, &setup_options).unwrap();
            assert!(
                empty.exists(),
                "{} deleted {}, which was there before it was",
                adapter.slug,
                empty.display()
            );
        }
    }
}

/// A file that is Leteo's, by its name or by what is still written in it.
fn files_naming_leteo(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            // The executable is the fixture's own, not something setup wrote.
            if path.starts_with(root.join("bin")) {
                continue;
            }
            if name.contains("leteo") || body.to_lowercase().contains("leteo") {
                found.push(
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .display()
                        .to_string(),
                );
            }
        }
    }
    found.sort();
    found
}

/// A binary npm is holding is not one to write into a configuration.
///
/// The npm wrapper leaves the real binary inside the package it downloaded it
/// for, and `setup` writes the path of whatever binary is running. Together
/// that puts `~/.npm/_npx/<hash>/node_modules/@asanabrial/leteo/vendor/…` into
/// an agent's configuration, which `npm cache clean` deletes and the next
/// version replaces. What follows is silent: the MCP server stops starting and
/// every hook fails without saying so, because that is what hooks do.
///
/// Measured before the guard existed, in a container: `setup claude-code
/// --hooks` through the wrapper wrote
/// `"command": "/home/u/npm/vendor/v0.1.1-x86_64-unknown-linux-gnu-leteo"`.
#[test]
fn setup_refuses_a_binary_the_package_manager_is_holding() {
    let temp = TempDir::new().unwrap();
    let managed = temp
        .path()
        .join(".npm")
        .join("_npx")
        .join("1a19a25e")
        .join("node_modules")
        .join("@asanabrial")
        .join("leteo")
        .join("vendor")
        .join("leteo");
    let held_by_npm = SetupOptions {
        executable: Some(managed.clone()),
        ..options(&temp)
    };

    let error = setup("claude-code", &held_by_npm)
        .expect_err("a path npm owns is refused")
        .to_string();
    assert!(
        error.contains("npm is holding") && error.contains("npx"),
        "the refusal names what is wrong and what to do instead: {error}"
    );

    // And nothing was written on the way to refusing, because a setup that
    // half-configures an agent and then errors is worse than one that does not
    // start: the file it left behind is the one nobody knows to look at.
    assert!(
        !temp.path().join(".claude.json").exists(),
        "the refusal came before any write"
    );

    // The ordinary path is untouched — this must not refuse a binary somebody
    // installed, which is every other run.
    setup("claude-code", &options(&temp)).expect("an installed binary still configures");
}
