//! Pi takes Leteo as a package, not as an MCP server.
//!
//! The package registers Leteo's MCP server itself, with `pi.registerMcpServer`,
//! and runs the lifecycle hooks. A server of the same name in `mcp.json` takes
//! precedence over that registration, so the entry an earlier setup left there
//! would freeze the old configuration, and with it the old exposure. Setup
//! therefore writes the package where Pi loads it from, lists it in Pi's
//! `settings.json`, and takes out the `leteo` entry from `mcp.json`.
//!
//! The package files are built into the binary. A binary installed with
//! `cargo install` has no `plugin/` directory beside it, and Pi has nothing to
//! fetch Leteo from: there is no npm package. The package is written under the
//! agent directory and listed there as a local path, which Pi loads without
//! copying (`packages.md`, "Choose a source").

use super::*;

/// What `plugin/pi` ships, as the pairs of file name and contents Pi reads.
const PACKAGE_JSON: &str = include_str!("../../plugin/pi/package.json");
const EXTENSION: &str = include_str!("../../plugin/pi/leteo.ts");
const EXTENSION_FILE: &str = "leteo.ts";
const PACKAGE_FILE: &str = "package.json";

/// The line in the extension that setup fills in. In the source it is empty and
/// the extension finds the binary on `PATH`; installed, it names the binary
/// that ran setup, as every other agent's configuration does, so a Pi started
/// from a launcher with a narrower `PATH` still finds it.
const INSTALLED_BINARY_LINE: &str = "const INSTALLED_BINARY = \"\"";

/// The line that holds the tool profile the extension asks `leteo mcp` for. The
/// source carries the default, which a test holds to [`DEFAULT_TOOLS`], and
/// setup writes what `--tools` asked for, as it does for every other agent.
const MCP_TOOLS_LINE: &str = "const MCP_TOOLS = \"agent\"";

/// Whether this adapter is Pi's, which takes a package where the others take a
/// server entry.
pub(super) fn is_pi(adapter: &AgentAdapter) -> bool {
    adapter.config_format == ConfigFormat::Json(McpFormat::Pi)
}

pub(super) fn package_dir(environment: &SetupEnvironment) -> PathBuf {
    environment.pi_agent_dir().join("packages").join("leteo")
}

pub(super) fn settings_path(environment: &SetupEnvironment) -> PathBuf {
    environment.pi_agent_dir().join("settings.json")
}

/// The files of the package, with the binary filled in.
fn package_files(executable: &Path, tools: &str) -> Result<[(&'static str, String); 2]> {
    let binary = serde_json::to_string(executable_string(executable)?)?;
    let tools = serde_json::to_string(tools)?;
    let extension = EXTENSION
        .replacen(
            INSTALLED_BINARY_LINE,
            &format!("const INSTALLED_BINARY = {binary}"),
            1,
        )
        .replacen(MCP_TOOLS_LINE, &format!("const MCP_TOOLS = {tools}"), 1);
    Ok([
        (PACKAGE_FILE, PACKAGE_JSON.to_owned()),
        (EXTENSION_FILE, extension),
    ])
}

/// Whether a `packages` entry of Pi's settings names the package directory.
///
/// Pi resolves a relative path against the directory the settings file is in,
/// and reads an entry as either a path or an object whose `source` is one, so
/// all of those spellings of the same directory are Leteo's.
fn names_package(entry: &Value, agent_dir: &Path, package: &Path) -> bool {
    let source = match entry {
        Value::String(source) => source,
        Value::Object(object) => match object.get("source").and_then(Value::as_str) {
            Some(source) => source,
            None => return false,
        },
        _ => return false,
    };
    let source = Path::new(source.trim());
    let resolved = if source.is_absolute() {
        source.to_path_buf()
    } else {
        agent_dir.join(source)
    };
    resolved.components().eq(package.components())
}

fn parse_settings(path: &Path, existing: Option<&[u8]>) -> Result<Value> {
    match existing {
        Some(content) if !content.iter().all(u8::is_ascii_whitespace) => {
            serde_json::from_slice(content).with_context(|| format!("parse {}", path.display()))
        }
        _ => Ok(Value::Object(Map::new())),
    }
}

fn packages_mut<'a>(root: &'a mut Value, path: &Path) -> Result<&'a mut Vec<Value>> {
    let object = root
        .as_object_mut()
        .with_context(|| format!("{} must contain a JSON object", path.display()))?;
    let packages = object
        .entry("packages")
        .or_insert_with(|| Value::Array(Vec::new()));
    if packages.is_null() {
        *packages = Value::Array(Vec::new());
    }
    packages
        .as_array_mut()
        .with_context(|| format!("packages in {} must contain an array", path.display()))
}

/// Pi's settings with the package listed, and nothing else touched.
fn register(
    path: &Path,
    existing: Option<&[u8]>,
    agent_dir: &Path,
    package: &Path,
) -> Result<String> {
    let mut root = parse_settings(path, existing)?;
    let packages = packages_mut(&mut root, path)?;
    // A file that already lists the package is handed back as it was. Writing
    // it out again would re-indent a file that is somebody else's and report a
    // change on a run that changed nothing.
    if packages
        .iter()
        .any(|entry| names_package(entry, agent_dir, package))
        && let Some(existing) = existing.and_then(|bytes| std::str::from_utf8(bytes).ok())
    {
        return Ok(existing.to_owned());
    }
    packages.push(Value::String(
        package
            .to_str()
            .with_context(|| format!("{} is not valid UTF-8", package.display()))?
            .to_owned(),
    ));
    to_json_like(
        existing
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or_default(),
        &root,
    )
}

/// Pi's settings with the package no longer listed.
fn deregister(path: &Path, existing: &[u8], agent_dir: &Path, package: &Path) -> Result<String> {
    let mut root = parse_settings(path, Some(existing))?;
    let Some(packages) = root.get_mut("packages").and_then(Value::as_array_mut) else {
        return Ok(String::from_utf8_lossy(existing).into_owned());
    };
    let before = packages.len();
    packages.retain(|entry| !names_package(entry, agent_dir, package));
    if packages.len() == before {
        return Ok(String::from_utf8_lossy(existing).into_owned());
    }
    to_json_like(std::str::from_utf8(existing).unwrap_or_default(), &root)
}

/// Whether Pi's settings list the package.
pub(super) fn is_registered(environment: &SetupEnvironment) -> bool {
    let Ok(Some(content)) = read_optional(&settings_path(environment)) else {
        return false;
    };
    let Ok(root) = serde_json::from_slice::<Value>(&content) else {
        return false;
    };
    let agent_dir = environment.pi_agent_dir();
    let package = package_dir(environment);
    root.get("packages")
        .and_then(Value::as_array)
        .is_some_and(|packages| {
            packages
                .iter()
                .any(|entry| names_package(entry, &agent_dir, &package))
        })
}

/// What `leteo setup pi` does: the package, its registration, and the old entry.
pub(super) fn setup(
    adapter: &AgentAdapter,
    options: &SetupOptions,
    environment: &SetupEnvironment,
    mcp_config: &Path,
) -> Result<SetupResult> {
    if options.install_instructions {
        anyhow::bail!(
            "{} has no instruction file; it receives the memory protocol at runtime",
            adapter.display_name
        );
    }
    if options.install_hooks {
        anyhow::bail!(
            "{} does not support Leteo lifecycle hooks yet",
            adapter.display_name
        );
    }
    let mut actions = Vec::new();

    // The entry an earlier setup wrote. A `mcp.json` server overrides an
    // extension's registration of the same name, so left in place it would keep
    // the old command and `codemode` exposure in force over the package's.
    if let Some(existing) = read_optional(mcp_config)?
        && names_leteo(&existing, adapter.config_format)
    {
        let desired = remove_json_server(&existing, McpFormat::Pi)?;
        actions.push(apply_content(
            mcp_config,
            Some(&existing),
            desired.as_bytes(),
            ActionKind::McpConfiguration,
            options.dry_run,
        )?);
    }

    let directory = package_dir(environment);
    for (name, content) in package_files(
        &environment.executable,
        options.tools.as_deref().unwrap_or(DEFAULT_TOOLS),
    )? {
        let path = directory.join(name);
        let existing = read_optional(&path)?;
        actions.push(apply_content(
            &path,
            existing.as_deref(),
            content.as_bytes(),
            ActionKind::Package,
            options.dry_run,
        )?);
    }

    let settings = settings_path(environment);
    let existing = read_optional(&settings)?;
    let desired = register(
        &settings,
        existing.as_deref(),
        &environment.pi_agent_dir(),
        &directory,
    )?;
    actions.push(apply_content(
        &settings,
        existing.as_deref(),
        desired.as_bytes(),
        ActionKind::PackageRegistration,
        options.dry_run,
    )?);

    Ok(SetupResult {
        agent: adapter.slug,
        dry_run: options.dry_run,
        actions,
    })
}

/// What `uninstall` adds for Pi, beside the `mcp.json` entry every agent loses.
pub(super) fn uninstall(
    options: &SetupOptions,
    environment: &SetupEnvironment,
) -> Result<Vec<SetupAction>> {
    let mut actions = Vec::new();

    let settings = settings_path(environment);
    if let Some(existing) = read_optional(&settings)? {
        let desired = deregister(
            &settings,
            &existing,
            &environment.pi_agent_dir(),
            &package_dir(environment),
        )?;
        actions.push(apply_content(
            &settings,
            Some(&existing),
            desired.as_bytes(),
            ActionKind::PackageRegistration,
            options.dry_run,
        )?);
    }

    // Only the files Leteo wrote. The directory goes when they were all that
    // was in it; anything else in there is somebody's, and so is the directory.
    let directory = package_dir(environment);
    for name in [PACKAGE_FILE, EXTENSION_FILE] {
        let path = directory.join(name);
        if path.exists() {
            actions.push(remove_file(&path, ActionKind::Package, options.dry_run)?);
        }
    }
    if !options.dry_run {
        let _ = fs::remove_dir(&directory);
        let _ = fs::remove_dir(directory.parent().unwrap_or(&directory));
    }
    Ok(actions)
}
