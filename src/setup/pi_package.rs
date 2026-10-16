//! Pi takes Leteo as a package, not as an MCP server.
//!
//! The package registers Leteo's MCP server itself, with `pi.registerMcpServer`,
//! and runs the lifecycle hooks. A server of the same name in `mcp.json` takes
//! precedence over that registration, so the entry an earlier setup left there
//! would freeze the old configuration, and with it the old exposure. Setup
//! therefore writes the package where Pi loads it from, lists it in Pi's
//! `settings.json`, and takes out the `leteo` entry from `mcp.json`.
//!
//! The package files are built into the binary, because a binary installed with
//! `cargo install` has no `plugin/` directory beside it. That is the default
//! route: the files are written under the agent directory and listed there as a
//! local path, which Pi loads without copying (`packages.md`, "Choose a
//! source"), and the extension is given the path of the binary that ran setup.
//! The same package is published to npm as `@asanabrial/leteo-pi`; installed
//! that way nothing has recorded a binary, and the extension falls back to
//! `LETEO_BIN` and then `PATH`.

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
    let Some(source) = entry_source(entry) else {
        return false;
    };
    let source = Path::new(source);
    let resolved = if source.is_absolute() {
        source.to_path_buf()
    } else {
        agent_dir.join(source)
    };
    resolved.components().eq(package.components())
}

fn entry_source(entry: &Value) -> Option<&str> {
    match entry {
        Value::String(source) => Some(source.trim()),
        Value::Object(object) => object.get("source")?.as_str().map(str::trim),
        _ => None,
    }
}

/// The entry that installs this package from npm, as the settings spell it.
///
/// Pi loads the first of two packages that register the `leteo` MCP server and
/// refuses the second at every start — measured on Pi 1.1.0 with this one from
/// npm beside the copy setup writes. The name is read from the manifest the
/// package ships rather than written out again. A version range follows it after
/// an `@`, and a name that only begins with it, `leteo-pixel`, is somebody else's.
fn npm_entry(packages: &[Value]) -> Option<String> {
    let manifest: Value = serde_json::from_str(PACKAGE_JSON).ok()?;
    let name = manifest.get("name")?.as_str()?;
    packages.iter().find_map(|entry| {
        let source = entry_source(entry)?;
        let rest = source.strip_prefix("npm:")?.strip_prefix(name)?;
        (rest.is_empty() || rest.starts_with('@')).then(|| source.to_owned())
    })
}

fn listed_npm_entry(path: &Path, existing: Option<&[u8]>) -> Result<Option<String>> {
    let root = parse_settings(path, existing)?;
    Ok(root
        .get("packages")
        .and_then(Value::as_array)
        .and_then(|packages| npm_entry(packages)))
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

/// Pi's settings listing both this package from npm and the copy setup wrote.
///
/// Setup refuses to add its copy beside the npm one, but `pi install` after a
/// setup adds the npm one without asking, so only `doctor` sees that order.
pub(super) fn duplicate_issue(environment: &SetupEnvironment) -> Option<String> {
    let settings = settings_path(environment);
    let content = read_optional(&settings).ok()??;
    let entry = listed_npm_entry(&settings, Some(&content)).ok()??;
    if !is_registered(environment) {
        return None;
    }
    Some(format!(
        "Pi lists Leteo twice in {}: `{entry}` and the package `leteo setup pi` wrote, so \
         it refuses the second at every start with `MCP server \"leteo\" is already \
         registered`. Run `pi remove {entry}`, or `leteo setup pi --uninstall`.",
        settings.display()
    ))
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
    // Everything is worked out before anything is written. The old entry going
    // and the package arriving are one change: done in the other order and
    // stopped by settings Pi cannot read, Pi would be left with no Leteo at all.
    let directory = package_dir(environment);
    let settings = settings_path(environment);
    let existing_settings = read_optional(&settings)?;
    if let Some(entry) = listed_npm_entry(&settings, existing_settings.as_deref())? {
        anyhow::bail!(
            "Pi already loads Leteo from npm (`{entry}` in {}), and a second copy would \
             be refused at every start with `MCP server \"leteo\" is already registered`. \
             Keep that one, or run `pi remove {entry}` and set up again.",
            settings.display()
        );
    }
    let desired_settings = register(
        &settings,
        existing_settings.as_deref(),
        &environment.pi_agent_dir(),
        &directory,
    )?;
    let files = package_files(
        &environment.executable,
        options.tools.as_deref().unwrap_or(DEFAULT_TOOLS),
    )?;

    // The entry an earlier setup wrote. A `mcp.json` server overrides an
    // extension's registration of the same name, so left in place it would keep
    // the old command and `codemode` exposure in force over the package's.
    let mcp = match read_optional(mcp_config)? {
        Some(existing) if names_leteo(&existing, adapter.config_format) => {
            let desired = remove_json_server(&existing, McpFormat::Pi)?;
            Some((existing, desired))
        }
        _ => None,
    };

    let mut actions = Vec::new();
    if let Some((existing, desired)) = mcp {
        actions.push(apply_content(
            mcp_config,
            Some(&existing),
            desired.as_bytes(),
            ActionKind::McpConfiguration,
            options.dry_run,
        )?);
    }
    for (name, content) in files {
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
    actions.push(apply_content(
        &settings,
        existing_settings.as_deref(),
        desired_settings.as_bytes(),
        ActionKind::PackageRegistration,
        options.dry_run,
    )?);

    Ok(SetupResult {
        agent: adapter.slug,
        dry_run: options.dry_run,
        actions,
    })
}

/// Whether Pi's settings can be read, asked before `uninstall` touches anything.
///
/// Removing the files and the old entry while the settings still list the
/// package would leave Pi pointing at a package that is not there, and settings
/// that cannot be parsed cannot be edited. So the run stops, names the file, and
/// leaves every other file as it found it.
pub(super) fn check_settings(environment: &SetupEnvironment) -> Result<()> {
    let settings = settings_path(environment);
    if let Some(existing) = read_optional(&settings)? {
        deregister(
            &settings,
            &existing,
            &environment.pi_agent_dir(),
            &package_dir(environment),
        )?;
    }
    Ok(())
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
