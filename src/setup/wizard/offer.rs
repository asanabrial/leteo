//! What the wizard can put on the table, worked out before anyone is asked.

use std::path::{Path, PathBuf};

use crate::engram;
use crate::setup::{SetupOptions, supported_agents};

/// Everything the wizard can offer, worked out before anyone is asked anything.
#[derive(Debug, Clone)]
pub struct Offer {
    /// An Engram installation worth adopting, if this store is still empty.
    pub engram: Option<engram::Installation>,
    /// Where an adoption would put the memories.
    pub database: PathBuf,
    /// Whether this store may already hold memories.
    ///
    /// Carried past the adoption question because a later one needs it too:
    /// choosing a language governs what is written from here on, and a store
    /// with memories already in it ends up holding two languages.
    ///
    /// True when the store could not be read, which is why it is "may". The
    /// warning it gates is worth showing to somebody whose store turns out to
    /// have been empty after all; withholding it from somebody with three
    /// thousand memories is not worth the tidiness.
    pub store_has_memories: bool,
    /// The agents that can be configured.
    pub agents: Vec<AgentChoice>,
    /// Where configuration is read from, and written back to.
    ///
    /// Carried so that applying uses the same paths the offer was read under.
    /// Detection has taken this as a parameter since a unit test broke over
    /// whichever agents the developer running it had installed; applying built
    /// its own defaults and so always wrote to the real machine, which left the
    /// half of the wizard that changes files as the half no test could reach.
    pub probe: SetupOptions,
}

/// An agent the wizard can offer to configure.
#[derive(Debug, Clone)]
pub struct AgentChoice {
    pub slug: String,
    pub display_name: String,
    /// Whether lifecycle hooks can be installed for it. Carried here because
    /// the wizard has to decide whether to ask about hooks at all before it
    /// calls setup, and asking for hooks an agent cannot take is an error.
    pub supports_hooks: bool,
    /// Whether this agent's configuration already registers Leteo.
    ///
    /// Read once, when the offer is built, and shown beside the name. Without
    /// it the screen offers twelve agents with twelve empty boxes whether none
    /// or all of them are set up, and cannot answer the question people arrive
    /// with: where did I install this?
    pub configured: bool,
}

/// Works out what can be offered, without asking anything.
///
/// `probe` says where to look for existing agent configuration. It is a
/// parameter rather than a default built in here because this reads real files:
/// with the paths hardcoded, a unit test's result depended on which agents the
/// developer running it happened to have installed, and one duly broke the
/// afternoon Leteo was uninstalled from Claude Code.
/// Builds what the wizard has to say, from what is actually on this machine.
///
/// `store_is_empty` means *known* to be empty, and the two decisions it drives
/// below lean opposite ways on purpose. A store nobody could read must not be
/// offered a migration over the top of it, and must be assumed to hold memories
/// worth warning about — so `false` covers both "there are memories" and "we
/// could not tell", which is the safe reading of each.
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

/// What to say about an Engram installation the wizard will not offer to adopt.
///
/// Adoption replaces the Leteo database rather than merging into it, so it is
/// only ever put as a question on an empty store. Staying silent is the worse
/// failure: somebody part-way through a migration runs `leteo setup`, sees no
/// mention of the Engram sitting right there, and concludes Leteo cannot find
/// it. Naming both counts also shows at a glance whether Engram has moved on.
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
