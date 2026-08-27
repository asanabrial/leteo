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
    let servers = super::open_servers_mut(object, format, path)?;
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

/// The comment line marking the block Leteo owns in the DeepSeek Harness patch
/// file.
///
/// Detection and removal both read this rather than re-parsing the YAML around
/// it: the file belongs to the harness, and the one thing that is unambiguously
/// Leteo is the block a past run wrote under its own marker, exactly like the
/// instruction-file markers in [`super::MEMORY_PROTOCOL_BEGIN`].
pub(super) const DSH_PATCH_MARKER: &str = "# leteo-mcp-client (managed by leteo setup)";

/// The YAML Leteo inserts into `$DSH_HOME/cordis.patch.yml` to register the
/// `leteo mcp` subprocess with the harness.
///
/// One row under an `insert:` patch directive. `serverName` must match the
/// harness's `[A-Za-z0-9_-]{1,32}` budget, which `leteo` does, and the tools it
/// names surface to the model as `mcp__leteo__<tool>`. The executable path and
/// the `--tools` argument are both single-quoted YAML scalars, where
/// backslashes (Windows) need no escaping.
///
/// Both go through the quoter, not just the path. `--tools` is free text off
/// the command line, and an apostrophe in it closed the scalar early and wrote
/// a patch file the harness cannot parse — and because this is the
/// machine-global layer, an unparseable row there costs every profile its
/// session, not just Leteo's server.
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

/// A YAML single-quoted scalar, where only a literal `'` needs escaping and a
/// `\\` is kept as written — the shape Windows paths must have here.
fn yaml_single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Whether a DeepSeek Harness patch file carries Leteo's block.
pub(super) fn dsh_names_leteo(text: &str) -> bool {
    text.lines().any(|line| line.trim() == DSH_PATCH_MARKER)
}

/// What a patch file holds, as far as appending a row to it is concerned.
///
/// The file's own header calls it "a top-level YAML array of loader patch
/// entries", and an array has two notations. Leteo writes the block one, so the
/// flow one has to be recognised rather than appended to: a document is one
/// node, and `[]` followed by `- insert:` is two.
#[derive(Debug, PartialEq, Eq)]
enum DshBody {
    /// Nothing, or nothing but comments. A block row may follow.
    NoEntries,
    /// `[]` — an array notation saying the same thing as no entries at all, but
    /// a node all the same, so it goes when the first row arrives.
    EmptyFlowArray,
    /// Rows already written the way Leteo writes them. Appending is safe.
    BlockArray,
    /// `[{...}]` with something in it, or a top-level mapping. Merging into
    /// either needs a parser this crate does not carry.
    Unappendable(&'static str),
}

/// Reads a patch file's shape without parsing it.
///
/// Only the first line that is neither blank nor a whole-line comment decides,
/// because that is the document's node and everything after it belongs to it.
fn dsh_body(text: &str) -> DshBody {
    let Some(first) = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
    else {
        return DshBody::NoEntries;
    };
    // `[]` on its own, with or without a comment after it.
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

/// Drops an `[]` node, keeping the comments above it.
///
/// Called only where [`dsh_body`] has already said there is one.
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

/// Inserts Leteo's `mcp-leteo` row into the harness patch file, replacing any
/// block Leteo previously wrote and preserving everything else by line.
///
/// Edited by line rather than parsed, because the file is ordinary YAML the
/// person writes by hand and this crate carries no YAML dependency. What is
/// owned is a block under a marker, so the render drops any prior block and
/// appends a fresh one; what is not owned — every other patch row, the user's
/// own `- insert:` lists — stays where it was.
///
/// # Why appending is not enough
///
/// The row is appended, and for most of a year the shape of what it was
/// appended to went unasked. Measured against a real harness home, a profile's
/// patch layer ships holding this:
///
/// ```text
/// # Your patch layer for this dsh profile, applied after every bundle layer:
/// # a top-level YAML array of loader patch entries (id-targeted config
/// # overrides, disables, and insert lists; `!!js` expressions allowed).
/// []
/// ```
///
/// `[]` is an empty array in flow notation and Leteo writes block notation, so
/// appending produced `[]` followed by `- insert:` — two nodes in one document,
/// which is not YAML at all. A parser stops at line 7 with "expected
/// `<document start>`". The file is the layer every profile composes, so what
/// that breaks is the harness, not the install: `leteo setup deepseek-harness`
/// against the shape the harness itself writes left the person unable to open a
/// session. The `[]` now goes when the first row arrives, which is the same
/// array said the other way.
///
/// A flow array with entries in it, or a mapping, is refused instead. Merging
/// into either needs the parser this crate deliberately does not carry, and
/// writing a document nothing can read is worse than saying so.
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

/// Drops the marker block Leteo wrote, keeping every other line as it was.
///
/// Refuses a file it cannot decode rather than reading it as empty. This took
/// the bytes with `unwrap_or_default`, so a `cordis.patch.yml` carrying one
/// Latin-1 byte — an accent from an editor that is not on UTF-8 — became the
/// empty string, stripped to nothing, and went back to disk as a zero-byte file
/// that `uninstall` reported as an ordinary update. That file is the
/// machine-global layer every profile composes, so what was lost is the
/// person's whole patch stack, not Leteo's row.
///
/// [`render_dsh_patch_config`], the install side of the very same file, has
/// always refused; this is the sibling that did not.
///
/// # Putting `[]` back
///
/// Install drops an `[]` when it writes the first row — see
/// [`render_dsh_patch_config`]. Taking that row out again would otherwise leave
/// a file holding nothing but its own header comments, and a document of only
/// comments is `null`, not the empty array the header says the file is. So
/// where the comments survive and no entry does, the `[]` goes back and the
/// file is the shape the harness shipped.
///
/// A file with nothing left in it at all is one Leteo created, and it is left
/// empty rather than given contents it never had.
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

/// Removes a complete Leteo block (its marker, the `- insert:` row and that
/// row's indented body), keeping every other line.
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
        // Take the marker's insert row and its indented body.
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
