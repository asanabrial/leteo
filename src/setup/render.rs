use super::*;

/// Merges Leteo's hooks into an agent settings file, replacing only the entries
/// Leteo owns and preserving every other setting and hook.
///
/// The registrations come from the agent, not from the global list: an agent
/// that cannot fire an event must not have it written where it would look
/// installed.
pub(super) fn render_hooks_config(
    path: &Path,
    existing: Option<&[u8]>,
    registrations: &[HookRegistration],
    executable: &Path,
) -> Result<String> {
    let mut root = match existing {
        Some(content) if !content.iter().all(u8::is_ascii_whitespace) => {
            serde_json::from_slice::<Value>(content)
                .with_context(|| format!("parse {}", path.display()))?
        }
        _ => Value::Object(Map::new()),
    };
    let object = root
        .as_object_mut()
        .with_context(|| format!("{} must contain a JSON object", path.display()))?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    if hooks.is_null() {
        *hooks = Value::Object(Map::new());
    }
    let hooks = hooks
        .as_object_mut()
        .with_context(|| format!("hooks in {} must contain a JSON object", path.display()))?;

    // Hook commands are handed to a shell, and always quoted.
    //
    // Quoting used to be conditional on the path containing a space, which read
    // as the careful thing and was wrong twice over. The shell that runs these
    // on Windows is bash, and bash treats a backslash as an escape: bare,
    // `C:\Users\me\AppData\Local\leteo\bin\leteo.exe` arrives as
    // `C:UsersmeAppDataLocalleteobinleteo.exe` and every hook fails with
    // "command not found". No space in sight — the default install path breaks
    // it. Quoting unconditionally costs two characters and is right everywhere.
    let executable = executable_string(executable)?;
    let command = format!("\"{executable}\"");
    // Leteo's own entries are pruned from every event present in the file, not
    // only from the ones about to be written. A hook that moves between events
    // — `session-stop` went from `Stop` to `SessionEnd` — would otherwise leave
    // its old registration behind on every machine that had already run setup,
    // and that stale copy would go on firing beside the new one forever.
    let present: Vec<String> = hooks.keys().cloned().collect();
    for event in present {
        let Some(entries) = hooks.get_mut(&event).and_then(Value::as_array_mut) else {
            continue;
        };
        entries.retain(|entry| !is_leteo_hook_entry(entry));
        // An event Leteo has abandoned and nobody else writes under is Leteo's
        // own leftover, so the key goes rather than sitting there empty.
        if entries.is_empty() && !registrations.iter().any(|item| item.event == event) {
            hooks.remove(&event);
        }
    }
    for registration in registrations {
        let entries = hooks
            .entry(registration.event.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
        if entries.is_null() {
            *entries = Value::Array(Vec::new());
        }
        entries.as_array_mut().with_context(|| {
            format!(
                "hooks.{} in {} must contain an array",
                registration.event,
                path.display()
            )
        })?;
    }
    for registration in registrations {
        let mut entry = Map::new();
        if let Some(matcher) = registration.matcher {
            entry.insert("matcher".to_owned(), Value::String(matcher.to_owned()));
        }
        entry.insert(
            "hooks".to_owned(),
            json!([{
                "type": "command",
                "command": format!("{command} hook {}", registration.slug),
                "timeout": registration.timeout_seconds,
            }]),
        );
        hooks
            .get_mut(registration.event)
            .and_then(Value::as_array_mut)
            .expect("hook event array was created above")
            .push(Value::Object(entry));
    }

    // Indented the way the file already was. Two spaces unconditionally meant
    // that adding one server to a four-space config rewrote every line of it.
    super::to_json_like(
        existing
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or_default(),
        &root,
    )
    .with_context(|| format!("serialize {}", path.display()))
}

/// Merges Leteo's hooks into ZCode's `config.json`, where they sit two keys
/// deeper than everywhere else: `hooks.events.<Event>`.
///
/// Two things differ from [`render_hooks_config`] beyond the nesting.
///
/// **The runner starts switched off.** Configuration-file hooks run only while
/// `hooks.enabled` is true — off by default, unlike a plugin bundle, which
/// enables the runner merely by existing. So Leteo switches it on, and refuses
/// where somebody has deliberately set it false: writing registrations into a
/// block the client will not read is a setup reporting success over files
/// nothing ever opens.
///
/// The rest holds all the way down. Commands are quoted unconditionally, each
/// event carries the matcher the binary needs told apart (`startup|clear`
/// against `compact`), and Leteo's own entries are pruned from every event
/// present before anything is written, so a moved registration cannot leave
/// its old copy firing beside the new one.
pub(super) fn render_zcode_hooks(
    path: &Path,
    existing: Option<&[u8]>,
    registrations: &[HookRegistration],
    executable: &Path,
) -> Result<String> {
    let mut root = match existing {
        Some(content) if !content.iter().all(u8::is_ascii_whitespace) => {
            serde_json::from_slice::<Value>(content)
                .with_context(|| format!("parse {}", path.display()))?
        }
        _ => Value::Object(Map::new()),
    };
    // Asked before anything is written — `setup` asks the same question earlier,
    // over the file as it arrived, precisely so this can never be reached after
    // a server entry went in. It stays here as the door this renderer closes on
    // its own authority.
    if super::zcode_hook_runner_disabled(&root) {
        anyhow::bail!(
            "{} sets hooks.enabled to false, which tells this client to run no \
             configuration hooks at all — Leteo's registrations would be written \
             and then ignored. Turn it back on there and run this again.",
            path.display()
        );
    }
    let object = root
        .as_object_mut()
        .with_context(|| format!("{} must contain a JSON object", path.display()))?;
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    if hooks.is_null() {
        *hooks = Value::Object(Map::new());
    }
    let hooks = hooks
        .as_object_mut()
        .with_context(|| format!("hooks in {} must contain a JSON object", path.display()))?;

    hooks.insert("enabled".to_owned(), Value::Bool(true));

    let events = hooks
        .entry("events")
        .or_insert_with(|| Value::Object(Map::new()));
    if events.is_null() {
        *events = Value::Object(Map::new());
    }
    let events = events.as_object_mut().with_context(|| {
        format!(
            "hooks.events in {} must contain a JSON object",
            path.display()
        )
    })?;

    let executable = executable_string(executable)?;
    let command = format!("\"{executable}\"");
    let present: Vec<String> = events.keys().cloned().collect();
    for event in present {
        let Some(entries) = events.get_mut(&event).and_then(Value::as_array_mut) else {
            continue;
        };
        entries.retain(|entry| !is_leteo_hook_entry(entry));
        if entries.is_empty() && !registrations.iter().any(|item| item.event == event) {
            events.remove(&event);
        }
    }
    for registration in registrations {
        let mut entry = Map::new();
        if let Some(matcher) = registration.matcher {
            entry.insert("matcher".to_owned(), Value::String(matcher.to_owned()));
        }
        entry.insert(
            "hooks".to_owned(),
            json!([{
                "type": "command",
                "command": format!("{command} hook {}", registration.slug),
                "timeout": registration.timeout_seconds,
            }]),
        );
        events
            .entry(registration.event.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("hook event array was created above")
            .push(Value::Object(entry));
    }

    super::to_json_like(
        existing
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or_default(),
        &root,
    )
    .with_context(|| format!("serialize {}", path.display()))
}

/// Asks the agent where its files go.
///
/// This used to be two `match` chains twelve arms long, one for configuration
/// and one for instructions, so every agent had a piece of itself here and
/// another in the registry. Now each agent answers for itself and this only
/// decides the order: the configuration first, because most agents derive the
/// instruction file from it.
pub(super) fn resolve_paths(adapter: &AgentAdapter, environment: &SetupEnvironment) -> AgentPaths {
    let mcp_config = (adapter.config_path)(environment);
    let instructions = adapter
        .instruction_path
        .map(|locate| locate(environment, &mcp_config));
    let hooks = adapter.hooks_path.map(|locate| locate(environment));

    AgentPaths {
        mcp_config,
        instructions,
        hooks,
    }
}

pub(super) fn render_json_config(
    path: &Path,
    existing: Option<&[u8]>,
    format: McpFormat,
    executable: &Path,
    tools: &str,
) -> Result<String> {
    let mut root = match existing {
        Some(content) if !content.iter().all(u8::is_ascii_whitespace) => {
            let content = if path
                .extension()
                .is_some_and(|extension| extension == "jsonc")
            {
                strip_jsonc(content)
            } else {
                content.to_vec()
            };
            serde_json::from_slice::<Value>(&content)
                .with_context(|| format!("parse {}", path.display()))?
        }
        _ => Value::Object(Map::new()),
    };

    let object = root
        .as_object_mut()
        .with_context(|| format!("{} must contain a JSON object", path.display()))?;
    let servers = super::open_servers_mut(object, format)?;
    servers.insert(
        SERVER_NAME.to_owned(),
        mcp_entry(format, executable, tools)?,
    );

    // Indented the way the file already was. Two spaces unconditionally meant
    // that adding one server to a four-space config rewrote every line of it.
    super::to_json_like(
        existing
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or_default(),
        &root,
    )
    .with_context(|| format!("serialize {}", path.display()))
}

/// Merges Leteo's hooks into a Codex `config.toml`.
///
/// Codex keeps hooks in the same file as the MCP server rather than in a
/// settings file of its own, and reads them as arrays of tables. So this edits
/// TOML the way [`render_codex_config`] does — by line, keeping everything it
/// does not recognise — rather than by parsing, which would reformat a file the
/// user writes by hand.
///
/// Leteo's entries are found by the command they run, not by their section
/// name. `[[hooks.SessionStart]]` is a heading the user writes under too, so
/// unlike `[mcp_servers.leteo]` the name cannot say whose an entry is. Each
/// entry spans a group of sections — the matcher table and the handler tables
/// under it — which is why the whole group is gathered before it is judged.
pub(super) fn render_codex_hooks(
    existing: Option<&[u8]>,
    registrations: &[HookRegistration],
    executable: &Path,
) -> Result<String> {
    let existing = match existing {
        Some(content) => std::str::from_utf8(content).context("Codex config is not valid UTF-8")?,
        None => "",
    };
    let normalized = existing.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    let kept = without_leteo_codex_hooks(&lines);

    // Quoted for the same reason the JSON hooks are: the command is handed to a
    // shell, and an unquoted Windows path loses its backslashes.
    let executable = executable_string(executable)?;
    let mut blocks = Vec::new();
    for registration in registrations {
        let command =
            serde_json::to_string(&format!("\"{executable}\" hook {}", registration.slug))
                .context("quote a Leteo hook command for TOML")?;
        let mut block = format!("[[hooks.{}]]\n", registration.event);
        if let Some(matcher) = registration.matcher {
            block.push_str(&format!("matcher = {}\n", serde_json::to_string(matcher)?));
        }
        block.push_str(&format!(
            "\n[[hooks.{}.hooks]]\ntype = \"command\"\ncommand = {command}\ntimeout = {}\n",
            registration.event, registration.timeout_seconds
        ));
        blocks.push(block);
    }

    let base = kept.join("\n");
    let base = base.trim_end();
    let hooks = blocks.join("\n");
    let desired = if base.is_empty() {
        hooks
    } else {
        format!("{base}\n\n{hooks}")
    };
    Ok(super::with_line_endings_of(existing, desired))
}

/// Drops the hook groups Leteo wrote, keeping every other line as it was.
///
/// Shared with uninstall, which has to take the hooks out as well as the
/// server: they sit in one file, and hooks left behind go on firing for a
/// binary the person just removed.
pub(super) fn without_leteo_codex_hooks<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        let Some(event) = hooks_group_heading(lines[index]) else {
            kept.push(lines[index]);
            index += 1;
            continue;
        };
        // The group runs to the next heading that is not one of this entry's
        // own handler tables.
        let start = index;
        index += 1;
        while index < lines.len() {
            let line = lines[index].trim();
            if is_toml_section(line) && !line.starts_with(&format!("[[hooks.{event}.")) {
                break;
            }
            index += 1;
        }
        let group = &lines[start..index];
        if !group.iter().any(|line| is_leteo_hook_command(line)) {
            kept.extend_from_slice(group);
        }
    }
    kept
}

/// The event named by a `[[hooks.<Event>]]` heading, if the line is one.
fn hooks_group_heading(line: &str) -> Option<&str> {
    let line = line.trim();
    let inner = line.strip_prefix("[[hooks.")?.strip_suffix("]]")?;
    // A handler table belongs to the group above it rather than opening one.
    (!inner.contains('.')).then_some(inner)
}

/// Whether a line runs one of Leteo's hooks, whatever path the binary had.
///
/// The binary is part of the question and not only the subcommand — see
/// [`super::runs_a_leteo_hook`], and the Codex file is the one where getting it
/// wrong drops a whole `[[hooks.<Event>]]` group belonging to somebody else.
fn is_leteo_hook_command(line: &str) -> bool {
    super::runs_a_leteo_hook(line)
}
