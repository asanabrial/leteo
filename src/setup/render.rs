use super::*;

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

    super::to_json_like(
        existing
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or_default(),
        &root,
    )
    .with_context(|| format!("serialize {}", path.display()))
}

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
    // Every event about to be written is checked before any of them is — the
    // same two passes [`render_hooks_config`] makes one level up. `entry` hands
    // back whatever the key already held, so an event this client left as
    // `null`, or as the bare object a hand-edited config can carry, went
    // straight into `as_array_mut().expect(...)` below and panicked. The prune
    // above steps over such a value rather than removing it, which is right —
    // it is not Leteo's — so refusing with the key named is the only answer
    // left, and it beats crashing over a config that was unusual, not broken.
    for registration in registrations {
        let entries = events
            .entry(registration.event.to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
        if entries.is_null() {
            *entries = Value::Array(Vec::new());
        }
        entries.as_array_mut().with_context(|| {
            format!(
                "hooks.events.{} in {} must contain an array",
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
        events
            .get_mut(registration.event)
            .and_then(Value::as_array_mut)
            .expect("hook event array was created or refused above")
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
    let servers = super::open_servers_mut(object, format, path)?;
    servers.insert(
        SERVER_NAME.to_owned(),
        mcp_entry(format, executable, tools)?,
    );

    super::to_json_like(
        existing
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or_default(),
        &root,
    )
    .with_context(|| format!("serialize {}", path.display()))
}

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

fn hooks_group_heading(line: &str) -> Option<&str> {
    let line = line.trim();
    let inner = line.strip_prefix("[[hooks.")?.strip_suffix("]]")?;
    (!inner.contains('.')).then_some(inner)
}

fn is_leteo_hook_command(line: &str) -> bool {
    super::runs_a_leteo_hook(line)
}

pub(super) const DSH_PATCH_MARKER: &str = "# leteo-mcp-client (managed by leteo setup)";

fn dsh_patch_block(executable: &str, tools: &str) -> String {
    let command = yaml_single_quoted(executable);
    // Built line by line (rather than by indented continuation) so the block
    // carries no trace of the Rust indentation it was written in.
    [
        DSH_PATCH_MARKER,
        "- insert:",
        &format!("    - id: mcp-{SERVER_NAME}"),
        "      name: '@deepseek-ai/dsh-mcp-client'",
        "      config:",
        "        transport: stdio",
        &format!("        serverName: {SERVER_NAME}"),
        &format!("        command: {command}"),
        &format!(
            "        args: ['mcp', {}]",
            yaml_single_quoted(&format!("--tools={tools}"))
        ),
        "        toolCallTimeoutMs: 60000",
        "        failOnStartupError: false",
    ]
    .join("\n")
        + "\n"
}

fn yaml_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) fn dsh_names_leteo(text: &str) -> bool {
    text.lines().any(|line| line.trim() == DSH_PATCH_MARKER)
}

#[derive(Debug, PartialEq, Eq)]
enum DshBody {
    NoEntries,
    EmptyFlowArray,
    BlockArray,
    Unappendable(&'static str),
}

fn dsh_body(text: &str) -> DshBody {
    let Some(first) = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
    else {
        return DshBody::NoEntries;
    };
    if let Some(rest) = first.strip_prefix("[]")
        && (rest.trim().is_empty() || rest.trim_start().starts_with('#'))
    {
        return DshBody::EmptyFlowArray;
    }
    if first == "-" || first.starts_with("- ") {
        return DshBody::BlockArray;
    }
    if first.starts_with('[') {
        return DshBody::Unappendable(
            "holds a flow-style array (`[…]`) with entries in it, and a row \
             cannot be appended to one by line",
        );
    }
    DshBody::Unappendable(
        "is a mapping rather than the top-level array of patch entries this \
         file is",
    )
}

fn drop_empty_flow_array(text: &str) -> String {
    let mut dropped = false;
    text.lines()
        .filter(|line| {
            if dropped {
                return true;
            }
            let trimmed = line.trim();
            let is_it = trimmed
                .strip_prefix("[]")
                .is_some_and(|rest| rest.trim().is_empty() || rest.trim_start().starts_with('#'));
            if is_it {
                dropped = true;
                return false;
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn render_dsh_patch_config(
    existing: Option<&[u8]>,
    executable: &str,
    tools: &str,
) -> Result<String> {
    let text = match existing {
        Some(content) => std::str::from_utf8(content)
            .map_err(|error| anyhow::anyhow!("patch file is not UTF-8: {error}"))?,
        None => "",
    };
    let normalized = text.replace("\r\n", "\n");
    let without_leteo = strip_dsh_block(&normalized);
    let without_leteo = match dsh_body(&without_leteo) {
        DshBody::NoEntries | DshBody::BlockArray => without_leteo,
        DshBody::EmptyFlowArray => drop_empty_flow_array(&without_leteo),
        DshBody::Unappendable(why) => anyhow::bail!(
            "the DeepSeek Harness patch file {why}. Leteo writes its row as a \
             block entry (`- insert:`), and adding one here would leave a \
             document the harness cannot parse — which would cost every profile \
             its session, not just Leteo's server. Make it a block-style array \
             and run this again."
        ),
    };
    let block = dsh_patch_block(executable, tools);
    let base = without_leteo.trim_end();
    let desired = if base.is_empty() {
        format!("{block}\n")
    } else {
        format!("{base}\n\n{block}\n")
    };
    Ok(super::with_line_endings_of(text, desired))
}

pub(super) fn remove_dsh_server(existing: &[u8]) -> Result<String> {
    let text =
        std::str::from_utf8(existing).context("DeepSeek Harness patch file is not valid UTF-8")?;
    let normalized = text.replace("\r\n", "\n");
    let stripped = strip_dsh_block(&normalized);
    let stripped = match dsh_body(&stripped) {
        DshBody::NoEntries if !stripped.trim().is_empty() => format!("{}\n[]", stripped.trim_end()),
        _ => stripped,
    };
    let body = stripped.trim_end();
    let desired = if body.is_empty() {
        String::new()
    } else {
        format!("{body}\n")
    };
    Ok(super::with_line_endings_of(text, desired))
}

fn strip_dsh_block(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept: Vec<&str> = Vec::with_capacity(lines.len());
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() != DSH_PATCH_MARKER {
            kept.push(lines[index]);
            index += 1;
            continue;
        }
        index += 1;
        if index < lines.len() && lines[index].trim_start().starts_with("- insert:") {
            index += 1;
            while index < lines.len() {
                let line = lines[index];
                let trimmed = line.trim_start();
                let blank = trimmed.is_empty();
                let indented = line.len() > trimmed.len();
                if blank || indented {
                    index += 1;
                } else {
                    break;
                }
            }
        }
    }
    kept.join("\n")
}
