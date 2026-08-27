use std::path::{Path, PathBuf};

use crate::engram;
use crate::setup::{SetupOptions, supported_agents};

#[derive(Debug, Clone)]
pub struct Offer {
    pub engram: Option<engram::Installation>,
    pub database: PathBuf,
    pub store_has_memories: bool,
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

pub fn offer(database: &Path, store_is_empty: bool, probe: &SetupOptions) -> Offer {
    let engram = store_is_empty
        .then(engram::default_database)
        .flatten()
        .and_then(|path| engram::inspect(&path).ok())
        .filter(|installation| !installation.is_empty());
    Offer {
        engram,
        database: database.to_path_buf(),
        store_has_memories: !store_is_empty,
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
         replaces the Leteo database rather than merging into it.\n\
         Run `leteo import --from-engram --dry-run` to see what it would take \
         over.",
        found.database.display(),
        found.observations,
    )
}
