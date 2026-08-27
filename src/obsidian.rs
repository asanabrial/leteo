//! Obsidian vault export.
//!
//! Writes each observation as a Markdown note with YAML frontmatter, plus hub
//! notes that turn sessions and topic clusters into graph nodes. The export is
//! idempotent: unchanged notes are left alone so Obsidian does not see a
//! filesystem event for every run.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::{Observation, Store, memory::normalize};

pub const VAULT_SUBDIRECTORY: &str = "leteo";
const SESSIONS_DIRECTORY: &str = "_sessions";
const TOPICS_DIRECTORY: &str = "_topics";
const MAX_SLUG_LENGTH: usize = 60;
/// Observations sharing a topic prefix before a hub note is worth creating.
const TOPIC_HUB_THRESHOLD: usize = 2;

/// How an existing `.obsidian/graph.json` is treated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum GraphConfig {
    /// Write the Leteo defaults only when the file is absent.
    #[default]
    Preserve,
    /// Always overwrite with the Leteo defaults.
    Force,
    /// Never read or write the file.
    Skip,
}

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    pub vault: PathBuf,
    pub project: Option<String>,
    pub limit: Option<usize>,
    pub graph_config: GraphConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ExportSummary {
    pub vault: String,
    pub notes_written: usize,
    pub notes_unchanged: usize,
    pub session_hubs: usize,
    pub topic_hubs: usize,
    pub graph_config_written: bool,
    /// Notes under `leteo/` that this export did not account for.
    ///
    /// A note is named from its memory's title, so a memory that is deleted —
    /// or renamed, which migration `0006` did to 898 session summaries at once
    /// — leaves its old note behind and writes a new one beside it. Nothing
    /// noticed, and the vault quietly filled with notes for memories the store
    /// no longer holds under names it no longer uses.
    ///
    /// Counted, not deleted. `leteo/` is Leteo's own subdirectory, but the
    /// vault is the person's and they may have linked to a note or edited it;
    /// removing files from it is their call to make with the list in hand.
    ///
    /// `None` when the export was narrowed by `--project` or `--limit`. Then
    /// most of the vault is unaccounted for *by design*, and a number would say
    /// something untrue.
    pub orphaned_notes: Option<usize>,
}

pub fn export(store: &Store, options: &ExportOptions) -> Result<ExportSummary> {
    let root = options.vault.join(VAULT_SUBDIRECTORY);
    // `--limit` is optional, and an absent optional limit means no limit —
    // which is what the README promises: *each* memory becomes a note.
    //
    // Passing the `None` straight through did not mean that.
    // `recent_observations` falls back to `max_context_results` when it is not
    // told, which is twenty and is right for assembling a session opening. Here
    // it silently capped the export: a store holding three thousand memories
    // wrote twenty notes, reported `notes_written: 20`, and exited a success.
    let limit = options.limit.unwrap_or(usize::MAX).min(i64::MAX as usize);
    let observations = store
        .recent_observations(options.project.as_deref(), Some(limit), true)
        .context("read observations for the Obsidian export")?;

    let mut summary = ExportSummary {
        vault: options.vault.to_string_lossy().into_owned(),
        ..ExportSummary::default()
    };
    let mut by_session: BTreeMap<String, Vec<NoteReference>> = BTreeMap::new();
    let mut by_topic: BTreeMap<String, Vec<NoteReference>> = BTreeMap::new();

    let mut written: BTreeSet<String> = BTreeSet::new();
    for observation in &observations {
        let slug = slugify(&observation.title, observation.id);
        written.insert(format!("{slug}.md"));
        let reference = NoteReference {
            slug: slug.clone(),
            kind: observation.kind.clone(),
        };
        if !observation.session_id.trim().is_empty() {
            by_session
                .entry(safe_component(&observation.session_id))
                .or_default()
                .push(reference.clone());
        }
        if let Some(topic_key) = observation
            .topic_key
            .as_deref()
            .filter(|topic| !topic.trim().is_empty())
        {
            by_topic
                .entry(topic_hub_name(topic_key))
                .or_default()
                .push(reference);
        }
        if write_if_changed(
            &root.join(format!("{slug}.md")),
            &observation_markdown(observation),
        )? {
            summary.notes_written += 1;
        } else {
            summary.notes_unchanged += 1;
        }
    }

    for (session, references) in &by_session {
        let path = root.join(SESSIONS_DIRECTORY).join(format!("{session}.md"));
        if write_if_changed(&path, &session_hub_markdown(session, references))? {
            summary.session_hubs += 1;
        }
    }
    for (topic, references) in &by_topic {
        if references.len() < TOPIC_HUB_THRESHOLD {
            continue;
        }
        let path = root.join(TOPICS_DIRECTORY).join(format!("{topic}.md"));
        if write_if_changed(&path, &topic_hub_markdown(topic, references))? {
            summary.topic_hubs += 1;
        }
    }

    summary.graph_config_written = write_graph_config(&options.vault, options.graph_config)?;
    // Only on a complete export: narrowed by project or by limit, everything
    // outside the narrowing is unaccounted for by design and a count would be
    // a number that means nothing.
    if options.project.is_none() && options.limit.is_none() {
        summary.orphaned_notes = Some(orphaned_notes(&root, &written)?);
    }
    Ok(summary)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NoteReference {
    slug: String,
    kind: String,
}

/// Notes sitting directly under `leteo/` that this export did not write.
///
/// The hub directories are skipped: a session or topic hub is Leteo's own
/// bookkeeping and is rewritten from whatever the export found, so one left
/// over is not a memory that went missing.
///
/// A vault that cannot be read counts as nothing orphaned rather than failing
/// the export. The notes are already written by this point, and refusing to
/// report a summary because a directory listing failed would throw away the
/// part that worked.
fn orphaned_notes(root: &Path, written: &BTreeSet<String>) -> Result<usize> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(0);
    };
    let mut orphaned = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.ends_with(".md") || written.contains(name) {
            continue;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_file()) {
            orphaned += 1;
        }
    }
    Ok(orphaned)
}

/// Converts a title into a filesystem-safe note name, always suffixed with the
/// observation identifier so two notes never collide.
pub fn slugify(title: &str, id: i64) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut previous_was_separator = false;
    for character in title.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_was_separator = false;
        } else if !previous_was_separator {
            slug.push('-');
            previous_was_separator = true;
        }
    }
    let slug = slug.trim_matches('-');
    let mut slug = slug.chars().take(MAX_SLUG_LENGTH).collect::<String>();
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        return format!("observation-{id}");
    }
    format!("{slug}-{id}")
}

pub fn observation_markdown(observation: &Observation) -> String {
    let project = observation.project.clone().unwrap_or_default();
    let topic_key = observation.topic_key.clone().unwrap_or_default();
    let mut note = String::from("---\n");
    note.push_str(&format!("id: {}\n", observation.id));
    note.push_str(&format!("type: {}\n", yaml_scalar(&observation.kind)));
    note.push_str(&format!("project: {}\n", yaml_scalar(&project)));
    note.push_str(&format!("scope: {}\n", yaml_scalar(&observation.scope)));
    note.push_str(&format!("topic_key: {}\n", yaml_scalar(&topic_key)));
    note.push_str(&format!(
        "session_id: {}\n",
        yaml_scalar(&observation.session_id)
    ));
    note.push_str(&format!(
        "created_at: {}\n",
        yaml_scalar(&observation.created_at)
    ));
    note.push_str(&format!(
        "updated_at: {}\n",
        yaml_scalar(&observation.updated_at)
    ));
    note.push_str(&format!("revision_count: {}\n", observation.revision_count));
    note.push_str("tags:\n");
    if !project.is_empty() {
        note.push_str(&format!("  - {}\n", yaml_scalar(&project)));
    }
    if !observation.kind.is_empty() {
        note.push_str(&format!("  - {}\n", yaml_scalar(&observation.kind)));
    }
    note.push_str(&format!(
        "aliases:\n  - {}\n",
        yaml_scalar(&observation.title)
    ));
    note.push_str("---\n\n");
    note.push_str(&format!("# {}\n\n", observation.title));
    note.push_str(&observation.content);
    note.push('\n');

    let mut links = Vec::new();
    if !observation.session_id.trim().is_empty() {
        links.push(format!(
            "*Session*: [[{}]]",
            safe_component(&observation.session_id)
        ));
    }
    if !topic_key.trim().is_empty() {
        links.push(format!("*Topic*: [[{}]]", topic_hub_name(&topic_key)));
    }
    if !links.is_empty() {
        note.push_str("\n---\n");
        for link in links {
            note.push_str(&link);
            note.push('\n');
        }
    }
    note
}

fn session_hub_markdown(session: &str, references: &[NoteReference]) -> String {
    let mut note = String::from("---\ntype: session-hub\n");
    note.push_str(&format!("session_id: {}\n", yaml_scalar(session)));
    note.push_str("tags:\n  - session\n---\n\n");
    note.push_str(&format!("# Session: {session}\n\n## Observations\n"));
    for reference in references {
        note.push_str(&format!("- [[{}]]\n", reference.slug));
    }
    note
}

fn topic_hub_markdown(topic: &str, references: &[NoteReference]) -> String {
    let mut note = String::from("---\ntype: topic-hub\n");
    note.push_str(&format!("topic_prefix: {}\n", yaml_scalar(topic)));
    note.push_str("tags:\n  - topic\n---\n\n");
    note.push_str(&format!("# Topic: {topic}\n\n## Related Observations\n"));
    for reference in references {
        note.push_str(&format!("- [[{}]] ({})\n", reference.slug, reference.kind));
    }
    note
}

/// Groups a topic key by everything before its last separator, so
/// `sdd/plugin/explore` clusters with `sdd/plugin/design`.
fn topic_hub_name(topic_key: &str) -> String {
    let topic_key = normalize::topic_key(Some(topic_key)).unwrap_or_default();
    let prefix = match topic_key.rfind('/') {
        Some(index) => &topic_key[..index],
        None => topic_key.as_str(),
    };
    safe_component(&prefix.replace('/', "--"))
}

/// Reduces an arbitrary identifier to one safe path component. Session
/// identifiers and topic keys come from agents, so a value such as `../escape`
/// must never reach the filesystem.
fn safe_component(value: &str) -> String {
    let mut safe = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
            safe.push(character);
        } else {
            safe.push('-');
        }
    }
    let safe = safe.trim_matches(|character| character == '.' || character == '-');
    if safe.is_empty() {
        "unnamed".to_owned()
    } else {
        safe.chars().take(MAX_SLUG_LENGTH).collect()
    }
}

/// Quotes a YAML scalar so titles with colons or quotes cannot break the
/// frontmatter block.
fn yaml_scalar(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', r"\\").replace('"', "\\\""))
}

/// Writes a file only when its content differs, keeping the export idempotent.
fn write_if_changed(path: &Path, content: &str) -> Result<bool> {
    if let Ok(existing) = std::fs::read_to_string(path)
        && existing == content
    {
        return Ok(false);
    }
    // The vault is the person's, and Obsidian may be watching the file while
    // this runs. A truncating write would show them a note that is half gone.
    crate::files::replace(path, content.as_bytes())
        .with_context(|| format!("write {}", path.display()))?;
    Ok(true)
}

const GRAPH_CONFIG: &str = r#"{
  "collapse-filter": false,
  "search": "",
  "showTags": false,
  "showAttachments": false,
  "hideUnresolved": false,
  "showOrphans": true,
  "collapse-color-groups": true,
  "colorGroups": [
    { "query": "path:leteo/_sessions", "color": { "a": 1, "rgb": 14736466 } },
    { "query": "path:leteo/_topics", "color": { "a": 1, "rgb": 13893887 } },
    { "query": "tag:#architecture", "color": { "a": 1, "rgb": 7935 } },
    { "query": "tag:#bugfix", "color": { "a": 1, "rgb": 16711680 } },
    { "query": "tag:#decision", "color": { "a": 1, "rgb": 65322 } },
    { "query": "tag:#pattern", "color": { "a": 1, "rgb": 16741120 } }
  ],
  "collapse-display": true,
  "showArrow": false,
  "textFadeMultiplier": 0,
  "nodeSizeMultiplier": 1,
  "lineSizeMultiplier": 1,
  "collapse-forces": false,
  "centerStrength": 0.5151475694444444,
  "repelStrength": 12.711805555555555,
  "linkStrength": 0.7292100694444444,
  "linkDistance": 207,
  "scale": 0.1,
  "close": false
}
"#;

fn write_graph_config(vault: &Path, mode: GraphConfig) -> Result<bool> {
    let path = vault.join(".obsidian").join("graph.json");
    match mode {
        GraphConfig::Skip => Ok(false),
        GraphConfig::Preserve if path.exists() => Ok(false),
        GraphConfig::Preserve | GraphConfig::Force => write_if_changed(&path, GRAPH_CONFIG),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::{
        memory::model::AddObservation,
        store::{Store, StoreConfig},
    };

    pub(super) fn store_with_notes() -> (TempDir, Store) {
        let temp = TempDir::new().unwrap();
        let mut store = Store::open(StoreConfig::new(temp.path().join("obsidian.db"))).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let add = |store: &mut Store, title: &str, topic: Option<&str>, kind: &str| {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: kind.to_owned(),
                    title: title.to_owned(),
                    content: format!("Body for {title}"),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: topic.map(str::to_owned),
                    prompt_sync_id: None,
                })
                .unwrap();
        };
        add(
            &mut store,
            "Auth: JWT rotation",
            Some("auth/jwt"),
            "decision",
        );
        add(
            &mut store,
            "Auth refresh window",
            Some("auth/refresh"),
            "architecture",
        );
        add(&mut store, "Standalone note", None, "bugfix");
        (temp, store)
    }

    #[test]
    fn slugs_are_filesystem_safe_bounded_and_unique() {
        assert_eq!(slugify("Fixed the auth bug!", 7), "fixed-the-auth-bug-7");
        assert_eq!(slugify("  Spaced   out  ", 1), "spaced-out-1");
        assert_eq!(slugify("", 3), "observation-3");
        assert_eq!(slugify("///", 4), "observation-4");
        assert_eq!(slugify("Título en español", 5), "t-tulo-en-espa-ol-5");
        let long = slugify(&"a".repeat(120), 9);
        assert_eq!(long, format!("{}-9", "a".repeat(MAX_SLUG_LENGTH)));
    }

    #[test]
    fn path_components_from_agents_cannot_escape_the_vault() {
        assert_eq!(safe_component("../../etc/passwd"), "etc-passwd");
        assert_eq!(safe_component("..\\..\\windows"), "windows");
        assert_eq!(safe_component("...."), "unnamed");
        assert_eq!(safe_component("normal-session_1"), "normal-session_1");
        assert_eq!(topic_hub_name("../secret/thing"), "secret");
        assert_eq!(topic_hub_name("auth/jwt"), "auth");
        assert_eq!(topic_hub_name("standalone"), "standalone");
        assert_eq!(topic_hub_name("sdd/plugin/explore"), "sdd--plugin");
    }

    #[test]
    fn frontmatter_quotes_values_that_would_break_yaml() {
        let temp = TempDir::new().unwrap();
        let mut store = Store::open(StoreConfig::new(temp.path().join("yaml.db"))).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let observation = store
            .add_observation(AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: r#"Use "quotes": and colons"#.to_owned(),
                content: "body".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap()
            .observation;

        let note = observation_markdown(&observation);

        assert!(note.contains(
            r#"aliases:
  - "Use \"quotes\": and colons""#
        ));
        assert!(note.contains("# Use \"quotes\": and colons"));
        assert!(note.contains("*Session*: [[s1]]"));
        assert!(!note.contains("*Topic*"));
    }

    #[test]
    fn export_writes_notes_hubs_and_graph_configuration_once() {
        let (_temp, store) = store_with_notes();
        let vault = TempDir::new().unwrap();
        let options = ExportOptions {
            vault: vault.path().to_owned(),
            project: Some("leteo".to_owned()),
            ..ExportOptions::default()
        };

        let first = export(&store, &options).unwrap();
        assert_eq!(first.notes_written, 3);
        assert_eq!(first.notes_unchanged, 0);
        assert_eq!(first.session_hubs, 1);
        assert_eq!(first.topic_hubs, 1, "auth/* clusters into one hub");
        assert!(first.graph_config_written);

        let root = vault.path().join(VAULT_SUBDIRECTORY);
        let hub = std::fs::read_to_string(root.join(SESSIONS_DIRECTORY).join("s1.md")).unwrap();
        assert!(hub.contains("# Session: s1"));
        assert!(hub.contains("- [[auth-jwt-rotation-1]]"));
        let topic = std::fs::read_to_string(root.join(TOPICS_DIRECTORY).join("auth.md")).unwrap();
        assert!(topic.contains("# Topic: auth"));
        assert!(topic.contains("(decision)"));
        assert!(topic.contains("(architecture)"));
        assert!(
            !root.join(TOPICS_DIRECTORY).join("standalone.md").exists(),
            "a single observation does not deserve a hub"
        );
        let graph =
            std::fs::read_to_string(vault.path().join(".obsidian").join("graph.json")).unwrap();
        assert!(graph.contains("path:leteo/_sessions"));

        let second = export(&store, &options).unwrap();
        assert_eq!(second.notes_written, 0);
        assert_eq!(second.notes_unchanged, 3);
        assert_eq!(second.session_hubs, 0);
        assert_eq!(second.topic_hubs, 0);
        assert!(!second.graph_config_written);
    }

    #[test]
    fn graph_configuration_modes_preserve_force_and_skip() {
        let (_temp, store) = store_with_notes();
        let vault = TempDir::new().unwrap();
        let graph_path = vault.path().join(".obsidian").join("graph.json");
        std::fs::create_dir_all(graph_path.parent().unwrap()).unwrap();
        std::fs::write(&graph_path, "{\"mine\":true}").unwrap();

        let preserved = export(
            &store,
            &ExportOptions {
                vault: vault.path().to_owned(),
                ..ExportOptions::default()
            },
        )
        .unwrap();
        assert!(!preserved.graph_config_written);
        assert_eq!(
            std::fs::read_to_string(&graph_path).unwrap(),
            "{\"mine\":true}"
        );

        let skipped = export(
            &store,
            &ExportOptions {
                vault: vault.path().to_owned(),
                graph_config: GraphConfig::Skip,
                ..ExportOptions::default()
            },
        )
        .unwrap();
        assert!(!skipped.graph_config_written);
        assert_eq!(
            std::fs::read_to_string(&graph_path).unwrap(),
            "{\"mine\":true}"
        );

        let forced = export(
            &store,
            &ExportOptions {
                vault: vault.path().to_owned(),
                graph_config: GraphConfig::Force,
                ..ExportOptions::default()
            },
        )
        .unwrap();
        assert!(forced.graph_config_written);
        assert!(
            std::fs::read_to_string(&graph_path)
                .unwrap()
                .contains("colorGroups")
        );
    }

    #[test]
    fn an_export_with_no_limit_writes_every_memory_and_not_a_context_budget() {
        // "Each memory becomes a Markdown note", says the README. `--limit` is
        // optional, and passing that `None` through to `recent_observations`
        // meant `max_context_results` — twenty. A store of three thousand wrote
        // twenty notes and reported it as a success.
        let temp = TempDir::new().unwrap();
        let mut store = Store::open(StoreConfig::new(temp.path().join("many.db"))).unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        // Three times the context budget that used to cap this silently.
        let wanted = StoreConfig::new("unused").max_context_results * 3;
        for index in 0..wanted {
            store
                .add_observation(AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: format!("Memory number {index}"),
                    content: format!("the body of memory number {index}"),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
        let vault = TempDir::new().unwrap();

        let summary = export(
            &store,
            &ExportOptions {
                vault: vault.path().to_path_buf(),
                project: Some("leteo".to_owned()),
                limit: None,
                graph_config: GraphConfig::Preserve,
            },
        )
        .unwrap();

        assert_eq!(summary.notes_written, wanted);
        let notes = std::fs::read_dir(vault.path().join(VAULT_SUBDIRECTORY))
            .unwrap()
            .flatten()
            .filter(|entry| entry.path().extension().is_some_and(|kind| kind == "md"))
            .count();
        assert_eq!(notes, wanted, "every memory has a note on disk");
    }

    #[test]
    fn an_export_that_was_given_a_limit_still_honours_it() {
        let (_temp, store) = store_with_notes();
        let vault = TempDir::new().unwrap();

        let summary = export(
            &store,
            &ExportOptions {
                vault: vault.path().to_path_buf(),
                project: Some("leteo".to_owned()),
                limit: Some(2),
                graph_config: GraphConfig::Preserve,
            },
        )
        .unwrap();

        assert_eq!(summary.notes_written, 2);
    }
}

#[cfg(test)]
mod orphan_tests {
    use super::tests::store_with_notes;
    use super::*;

    /// A renamed memory writes a new note and leaves the old one behind.
    ///
    /// The note's name comes from its memory's title, so anything that changes
    /// a title changes the file. Migration `0006` changed 898 of them in one
    /// pass — every session summary, which had all been called
    /// `Session summary: <project>`. Nothing noticed: the export reported only
    /// what it wrote, and the vault filled with notes for names no memory uses.
    ///
    /// Counted rather than deleted. The vault is the person's and they may have
    /// linked to a note; the list is theirs to act on.
    #[test]
    fn a_note_left_by_a_renamed_memory_is_counted_and_not_removed() {
        let (temp, mut store) = store_with_notes();
        let vault = temp.path().join("vault");
        let options = ExportOptions {
            vault: vault.clone(),
            ..ExportOptions::default()
        };

        let first = export(&store, &options).unwrap();
        assert!(first.notes_written > 0);
        assert_eq!(
            first.orphaned_notes,
            Some(0),
            "a fresh vault holds nothing but what was just written"
        );

        let id = store
            .recent_observations(None, Some(1), true)
            .unwrap()
            .first()
            .expect("the fixture wrote memories")
            .id;
        store
            .update_observation(
                id,
                crate::memory::model::UpdateObservation {
                    title: Some("A completely different title now".to_owned()),
                    ..Default::default()
                },
            )
            .unwrap();

        let second = export(&store, &options).unwrap();
        assert_eq!(
            second.orphaned_notes,
            Some(1),
            "the note under the old name is still there and has to be said"
        );
        // And still there: this reports, it does not tidy.
        let remaining = std::fs::read_dir(vault.join(VAULT_SUBDIRECTORY))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".md"))
            .count();
        assert_eq!(remaining, first.notes_written + 1);

        // Narrowed exports say nothing rather than something untrue: outside
        // the narrowing, everything looks orphaned.
        let narrowed = export(
            &store,
            &ExportOptions {
                vault,
                limit: Some(1),
                ..ExportOptions::default()
            },
        )
        .unwrap();
        assert_eq!(narrowed.orphaned_notes, None);
    }
}
