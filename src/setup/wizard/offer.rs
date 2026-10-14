use std::path::{Path, PathBuf};

use crate::engram;
use crate::setup::{SetupOptions, supported_agents};

#[derive(Debug, Clone)]
pub struct Offer {
    pub engram: Option<engram::Installation>,
    pub database: PathBuf,
    /// The live memories the store already holds, per recorded stem language
    /// code, or `None` when the store could not be read at all.
    ///
    /// The memory-language step is the only reader, and it reads this to say
    /// what a change of setting would leave behind. `None` is not "empty": a
    /// store nobody could read is not one with nothing in it, and that step
    /// warns without a number rather than staying quiet about memories it
    /// cannot see.
    pub memories_by_language: Option<Vec<(String, i64)>>,
    pub agents: Vec<AgentChoice>,
    pub probe: SetupOptions,
}

#[derive(Debug, Clone)]
pub struct AgentChoice {
    pub slug: String,
    pub display_name: String,
    pub supports_hooks: bool,
    pub configured: bool,
}

pub fn offer(
    database: &Path,
    store_is_empty: bool,
    memories_by_language: Option<Vec<(String, i64)>>,
    probe: &SetupOptions,
) -> Offer {
    let engram = store_is_empty
        .then(engram::default_database)
        .flatten()
        .and_then(|path| engram::inspect(&path).ok())
        .filter(|installation| !installation.is_empty());
    Offer {
        engram,
        database: database.to_path_buf(),
        memories_by_language,
        probe: probe.clone(),
        agents: supported_agents()
            .iter()
            .map(|agent| AgentChoice {
                configured: crate::setup::is_configured(agent.slug, probe),
                slug: agent.slug.to_owned(),
                display_name: agent.display_name.to_owned(),
                supports_hooks: agent.supports_hooks(),
            })
            .collect(),
    }
}

pub fn adoption_note(found: &engram::Installation, held: i64) -> String {
    format!(
        "Engram is installed at {}, holding {} memories.\n\
         Leteo already holds {held}, so adopting is not offered below: it \
         refuses a store that already holds memories rather than merging into \
         it.\n\
         Run `leteo import --from-engram --dry-run` to see what it would take \
         over.",
        found.database.display(),
        found.observations,
    )
}
