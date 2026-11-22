//! The memory blob an agent is handed when a session starts.
//!
//! Shared by `leteo context` and by the session-start and compaction hooks, so
//! what an agent reads at the top of a conversation is written in one place
//! rather than once per surface that hands it over.

use std::collections::BTreeMap;

use crate::memory::model::{Caveat, Observation, Prompt, SessionSummary};

/// How many of the most recent memories `mem_context` hands over with their
/// content.
///
/// The rest arrive as titles. The markdown opening block no longer previews
/// any: it is titles only, and the reason is in `render_context`.
pub(crate) const DETAILED: usize = 5;

// How many memories an opening context names is no longer a constant. It began
// in the hook, moved here when `mem_context` started following the same rule,
// and is now a setting — `settings::ContextSize`, which carries the measurement
// that decides the three sizes. Both surfaces read it, so somebody who changes
// it is answered by the next session rather than by reinstalling anything.

use crate::memory::normalize::TITLE_CHARS;

/// How much of what a session was for its line carries.
///
/// It was 200, written out three times, and 200 sits just above the *median*:
/// over 338 session summaries on a real store the median is 182, the p90 is 249
/// and the p99 is 304, so **37% of the lines lost their end** — in the section
/// whose whole job is to say what each session was about. At this bound it is
/// 1%, and the same argument that put `TITLE_CHARS` past the p99 applies here
/// unchanged; nobody had made it twice.
///
/// Past the p99 rather than near it, and that distinction cost a commit: the
/// first attempt at this line was 300, chosen because it is a round number
/// while the measurement sat four characters above it. The guard below is
/// written against the p99 and caught it.
///
/// It costs about a hundred and thirty bytes across the five sessions a block
/// lists, because only the third that were being cut grow at all.
pub(crate) const SESSION_LINE_CHARS: usize = 320;

/// How much of a question its line carries.
///
/// The same number the other budget used to be, kept apart because the reason
/// is the opposite one. A prompt is whatever somebody typed and people paste:
/// the median is 67 characters and the p90 is 414, which is two populations
/// rather than one. Cutting here is the point — 18% of prompts are cut at this
/// bound and that is 18% of pasted files not spent on an opening block.
pub(crate) const PROMPT_LINE_CHARS: usize = 200;

/// Gathers a session's opening context and renders it.
///
/// Both surfaces that hand context to an agent — `leteo context` and the
/// session-start hook — used to do this themselves: ask for pinned memories,
/// ask for recent ones, drop the pinned from the recent, fold the summaries,
/// truncate, render. Six steps in a fixed order, written twice, in two files,
/// with the order mattering (folding before truncating, or the summaries eat
/// the budget). That is a rule about what context *is*, so it belongs here with
/// the other one rather than once per caller.
pub fn assemble(
    store: &crate::store::Store,
    project: Option<&str>,
    scope: Option<&str>,
    memories: usize,
    budget_bytes: usize,
) -> Result<String, crate::StoreError> {
    assemble_counted(store, project, scope, memories, budget_bytes).map(|(context, _)| context)
}

/// [`assemble`], plus how many memories the context it built actually lists.
///
/// The count is not the `memories` that were asked for, and a caller that
/// reported that number instead would be telling somebody about memories that
/// are not on their screen: pinned entries are listed on top of the budget,
/// session summaries are folded onto their sessions and stop counting as
/// memories, and a young project has fewer than were requested.
pub fn assemble_counted(
    store: &crate::store::Store,
    project: Option<&str>,
    scope: Option<&str>,
    memories: usize,
    budget_bytes: usize,
) -> Result<(String, usize), crate::StoreError> {
    assemble_listed(store, project, scope, memories, budget_bytes)
        .map(|(context, listed)| (context, listed.memories.len()))
}

/// What a rendered block carries, by name rather than by number.
///
/// The per-prompt hint has to know which memories the opening block already put
/// in front of the agent, and which caveats it printed under them. Re-deriving
/// that from the same queries would name the memories the byte budget *asked*
/// for, and the budget cuts memories from the block: one that was cut was not
/// shown and must not be recorded as shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listed {
    /// Pinned memories first, then the titled ones, as the block lists them.
    pub memories: Vec<i64>,
    /// One key per caveat line printed, in the form [`Caveat::key`] writes.
    pub caveats: Vec<String>,
}

/// [`assemble_counted`], with the memories and caveats the block lists in place
/// of how many there are.
pub fn assemble_listed(
    store: &crate::store::Store,
    project: Option<&str>,
    scope: Option<&str>,
    memories: usize,
    budget_bytes: usize,
) -> Result<(String, Listed), crate::StoreError> {
    let mut sessions = store.recent_sessions(project, Some(RECENT_SESSIONS))?;
    // With a ceiling of its own, the same one the recent budget takes. See
    // `pinned_observations`: without it a project with 360 pinned memories put
    // all 360 into every session opening — 47 KB — and nobody could ask for
    // fewer, because this surface takes no limit from anyone.
    //
    // "The same one" was not true. This said `ContextSize::Deep`, which is the
    // deepest anybody is ever configured to open with, while the recent budget
    // is whatever the caller or the setting asked for — so the two matched only
    // on `deep`. Somebody who chose `slim`, whose whole purpose is a small
    // opening, got twenty recent memories and eighty pinned ones: driven
    // against a copy of a real store with a hundred pins, `slim` answered with
    // a hundred memories and 75 KB, and `mem_context` asked for five answered
    // with eighty-five.
    //
    // A pin is a deliberate act and trimming one is not free, which is why
    // `pinned_omitted` exists and why the opening block says how many are not
    // shown. What is not defensible is a setting that says twenty and a reply
    // that carries a hundred.
    let (pinned, pinned_omitted) = store.pinned_observations(project, scope, memories)?;
    // Exactly what will be shown, because `recent_memories` says the three
    // conditions in SQL — not pinned, not a summary, in this scope — and lets
    // SQLite count the limit after them.
    //
    // This is where a `memories * 4` used to be, and the note that stood here
    // explained how to saturate that multiplication safely: `memories` arrives
    // from outside, `leteo context --limit` and `mem_context`'s `limit` both
    // reach here unbounded, and a plain `* 4` panicked a debug build and
    // silently wrapped a release one. The multiplication is gone, so the
    // hazard is gone with it — but the shape of that bug is worth keeping in
    // mind by anyone who reintroduces arithmetic on this number.
    let observations = store.recent_memories(project, scope, memories)?;
    // The summaries the sessions above are missing, asked for by session.
    //
    // These two used to be one query: fetch four times the budget and sort out
    // afterwards what was a memory, what was pinned, what belonged to this
    // scope, and what was a summary to fold. Four times over is a guess, and it
    // was wrong at both ends — it read 360KB of memory bodies to show 175KB of
    // them, and it still lost the summary of any session older than the window,
    // which on a real store left 3 of 19 recent sessions listed as a name and a
    // date with nothing about what they were for.
    let summaries = store.session_summaries(&session_ids(&sessions))?;
    fold_session_summaries(&mut sessions, summaries);
    let prompts = store.recent_distinct_prompts(project, Some(RECENT_PROMPTS))?;
    // What the graph says about every memory about to be listed. This is the
    // larger of the two surfaces that hand memories over — fifty of them at a
    // session opening against three on a prompt — so a memory a later one
    // overturned does the most damage here.
    let named: Vec<String> = pinned
        .iter()
        .chain(observations.iter())
        .map(|observation| observation.sync_id.clone())
        .collect();
    //
    // Not fatal. The annotation is worth having and the context is worth far
    // more, so a store that cannot answer this costs the caveats rather than
    // the whole opening context — which is what `?` here would have cost.
    let caveats = store.caveats_for(&named).unwrap_or_default();
    // The count comes back with the text, from the same render, because the
    // byte budget can drop memories: a count taken before the cut is a number
    // the block does not carry, and this is the surface whose caller reports
    // that number to a person (`hooks/context.rs`).
    Ok(format_context_listed(
        &sessions,
        &prompts,
        &pinned,
        pinned_omitted,
        &observations,
        &caveats,
        Frame {
            project,
            budget_bytes,
        },
    ))
}

/// How many memories a context carries when the caller does not say.
///
/// From the setting, because three surfaces build this same context and two of
/// them asked. `leteo context` used a constant twenty while the session-start
/// hook and `mem_context` both read `context_size`, whose default is thirty —
/// so on an untouched installation the terminal showed 40% of what the agent
/// was handed, and `leteo setup --context deep` moved one of them and not the
/// other. Twenty was Slim's number, which is why nobody noticed: the two agreed
/// exactly when somebody had chosen the smallest size.
///
/// Read per call rather than captured, for the reason the language is: somebody
/// who changes the size is answered by the next call.
pub fn default_memories(store: &crate::store::Store) -> usize {
    crate::settings::load_beside(store.database_path())
        .context_size()
        .memories()
}

/// The byte budget a context carries when the caller does not say.
///
/// The same setting that decides the memory count decides the size, so a
/// caller that names neither gets a block whose count and whose ceiling belong
/// to the same [`crate::settings::ContextSize`].
pub fn default_bytes(store: &crate::store::Store) -> usize {
    crate::settings::load_beside(store.database_path())
        .context_size()
        .bytes()
}

pub const RECENT_SESSIONS: usize = 5;
pub const RECENT_PROMPTS: usize = 10;

/// Moves session summaries out of the memory list and onto their sessions.
///
/// A summary is saved twice over: `mem_session_summary` writes it as an
/// observation *and* onto the session row. The observation then competes with
/// real memories for room — on a live project, thirty of the fifty most recent
/// were summaries, so the context spent most of itself saying "there was a
/// session" while the decisions and bug fixes it was meant to carry were pushed
/// out. Meanwhile the sessions list showed names and dates with no summary at
/// all, because that column is often empty.
///
/// Folding them back fixes both ends at once: the sessions list gains what each
/// session was actually for, and the memory list becomes memories.
///
pub fn fold_session_summaries(sessions: &mut [SessionSummary], summaries: Vec<Observation>) {
    for session in sessions.iter_mut() {
        if session
            .summary
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            continue;
        }
        if let Some(found) = summaries
            .iter()
            .find(|summary| summary.session_id == session.id)
        {
            session.summary = Some(goal_of(&found.content));
        }
    }
}

fn session_ids(sessions: &[SessionSummary]) -> Vec<String> {
    sessions.iter().map(|session| session.id.clone()).collect()
}

/// What a session was for, from a structured summary.
///
/// `mem_session_summary` asks for a document — Goal, Discoveries, Accomplished,
/// Next Steps — and the whole thing runs to a page. Beside a session in a list,
/// the goal is the part that says which session this was; truncating the
/// document instead would spend the same room printing the words "## Goal".
fn goal_of(summary: &str) -> String {
    let after_heading = summary
        .split_once("## Goal")
        .map_or(summary, |(_, rest)| rest);
    let goal = after_heading
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("");
    if goal.is_empty() {
        return truncate(summary.trim(), SESSION_LINE_CHARS);
    }
    truncate(goal, SESSION_LINE_CHARS)
}

/// What frames a block rather than fills it: where it is scoped, and how large
/// it may be.
///
/// Together because they arrive together and `format_context` was at the
/// argument count clippy stops at.
#[derive(Clone, Copy)]
pub struct Frame<'a> {
    /// The project the block is scoped to. Lines of that project do not name
    /// it; with none, every line does.
    pub project: Option<&'a str>,
    pub budget_bytes: usize,
}

/// Renders the opening context.
///
/// `caveats` is what the relation graph says about the memories being listed,
/// keyed by `sync_id`. An empty map renders exactly what this rendered before
/// the graph was read, which is what the tests below pass.
pub fn format_context(
    sessions: &[SessionSummary],
    prompts: &[Prompt],
    pinned: &[Observation],
    pinned_omitted: usize,
    observations: &[Observation],
    caveats: &BTreeMap<String, Vec<Caveat>>,
    frame: Frame,
) -> String {
    format_context_counted(
        sessions,
        prompts,
        pinned,
        pinned_omitted,
        observations,
        caveats,
        frame,
    )
    .0
}

/// [`format_context`], plus how many memories the block it built actually names.
///
/// The count and the text have to come out of the same render. The byte budget
/// can drop memories, so counting them before the cut would report a number the
/// block does not carry — the disagreement between what the agent reads and
/// what the person is told that `hooks/context.rs` exists to prevent.
pub fn format_context_counted(
    sessions: &[SessionSummary],
    prompts: &[Prompt],
    pinned: &[Observation],
    pinned_omitted: usize,
    observations: &[Observation],
    caveats: &BTreeMap<String, Vec<Caveat>>,
    frame: Frame,
) -> (String, usize) {
    let (context, listed) = format_context_listed(
        sessions,
        prompts,
        pinned,
        pinned_omitted,
        observations,
        caveats,
        frame,
    );
    (context, listed.memories.len())
}

/// [`format_context_counted`], naming the memories instead of counting them.
///
/// The count is the length of the list, so the two cannot disagree.
pub fn format_context_listed(
    sessions: &[SessionSummary],
    prompts: &[Prompt],
    pinned: &[Observation],
    pinned_omitted: usize,
    observations: &[Observation],
    caveats: &BTreeMap<String, Vec<Caveat>>,
    frame: Frame,
) -> (String, Listed) {
    let Frame {
        project,
        budget_bytes,
    } = frame;
    // A session with no summary is a date and nothing else: it cannot tell
    // anybody what the work was for, and on a real store one such line was a
    // bare `- (2026-08-01 23:14)` among five. The test comes after
    // `fold_session_summaries`, so a session whose summary lives in a
    // `session_summary` memory is kept. Decided here, before the count is
    // taken, so the number of sessions shown is the number of lines printed.
    let sessions: Vec<&SessionSummary> = sessions
        .iter()
        .filter(|session| {
            session
                .summary
                .as_deref()
                .is_some_and(|summary| !summary.trim().is_empty())
        })
        .collect();
    if sessions.is_empty() && prompts.is_empty() && pinned.is_empty() && observations.is_empty() {
        return (String::new(), Listed::default());
    }

    let mut shown = Shown {
        sessions: sessions.len(),
        prompts: prompts.len(),
        pinned: pinned.len(),
        listed: (0..observations.len()).collect(),
    };
    // The spelling the sessions carry, which `recent_sessions` narrowed by.
    let own_project = project.map(crate::memory::normalize::project);
    let sections = Sections {
        sessions: &sessions,
        prompts,
        pinned,
        pinned_omitted,
        observations,
        caveats,
        own_project: own_project.as_deref(),
    };
    // Render, and when the block is over budget drop the least important thing
    // still in it and render again. Dropping one item at a time rather than
    // cutting the finished string is what keeps the cut on a line boundary and
    // what lets the count of what was left out be exact.
    loop {
        let rendered = render_context(&sections, &shown, budget_bytes);
        if rendered.len() <= budget_bytes || !shown.drop_one() {
            let named: Vec<&Observation> = pinned[..shown.pinned]
                .iter()
                .chain(shown.listed.iter().map(|&index| &observations[index]))
                .collect();
            let listed = Listed {
                memories: named.iter().map(|observation| observation.id).collect(),
                caveats: named
                    .iter()
                    .flat_map(|observation| {
                        caveats
                            .get(&observation.sync_id)
                            .into_iter()
                            .flatten()
                            .map(|caveat| caveat.key(observation.id))
                    })
                    .collect(),
            };
            return (rendered, listed);
        }
    }
}

/// How much of each section a budgeted block still carries.
///
/// The newest memories are the ones the work is likely continuing from, so a
/// block that has to lose something loses it from the oldest end, and the
/// sections that only frame the memories give way before the memories do:
/// prompts, then all but the newest session, then the remembered titles, then
/// that last session, and the pinned memories last — a pin is a deliberate
/// act, and dropping one in silence is the failure `pinned_omitted` exists to
/// prevent.
///
/// The memories went first until the figure was measured: on the four largest
/// projects of a copy of a real store, every block was cut at 4,000 bytes and
/// left out 15, 10, 14 and 10 of thirty memories, while prompts took 1,301
/// bytes of one block and four-month-old sessions much of another. A size that
/// names thirty memories has to show thirty before it shows anything older.
#[derive(Default)]
struct Shown {
    sessions: usize,
    prompts: usize,
    pinned: usize,
    /// Indices into `observations`, newest first, shown as a title only.
    listed: Vec<usize>,
}

impl Shown {
    /// Drops one item, the least important the block still carries.
    fn drop_one(&mut self) -> bool {
        if self.prompts > 0 {
            self.prompts -= 1;
            return true;
        }
        if self.sessions > 1 {
            self.sessions -= 1;
            return true;
        }
        if self.listed.pop().is_some() {
            return true;
        }
        if self.sessions > 0 {
            self.sessions -= 1;
            return true;
        }
        if self.pinned > 0 {
            self.pinned -= 1;
            return true;
        }
        false
    }
}

/// The sections a context is rendered from.
struct Sections<'a> {
    /// Only sessions that carry a summary; see `format_context_counted`.
    sessions: &'a [&'a SessionSummary],
    prompts: &'a [Prompt],
    pinned: &'a [Observation],
    pinned_omitted: usize,
    observations: &'a [Observation],
    caveats: &'a BTreeMap<String, Vec<Caveat>>,
    /// The project the block is scoped to, as the store spells it.
    own_project: Option<&'a str>,
}

/// Renders the block the current [`Shown`] allows, and says what it left out.
fn render_context(sections: &Sections, shown: &Shown, budget_bytes: usize) -> String {
    let Sections {
        sessions,
        prompts,
        pinned,
        pinned_omitted,
        observations,
        caveats,
        own_project,
    } = *sections;
    let mut context = String::from("## Memory from Previous Sessions\n\n");
    if shown.sessions > 0 {
        context.push_str("### Recent Sessions\n");
        for session in &sessions[..shown.sessions] {
            let summary = session
                .summary
                .as_deref()
                .map(|summary| format!(": {}", truncate(summary, SESSION_LINE_CHARS)))
                .unwrap_or_default();
            // When the session last did something, not when it opened.
            //
            // Every listing of sessions orders by the last activity, and this
            // printed the start: a list sorted by one date and labelled with
            // another, which reads as a list that is not sorted at all. On a
            // real store the opening context came out 08-05, 07-28, 07-31,
            // 07-27 — and the 07-28 line was the session that had saved a
            // memory twenty minutes earlier, 148 of them since July, while the
            // 08-05 line held two. An agent reading the dates concluded the
            // material was a week old and the freshest line was the stalest.
            //
            // Neither the project nor the count is printed where it carries
            // nothing: a block scoped to one project said that project's name
            // on every line, and `[N observations]` is a number nobody
            // chooses a session by — the summary is what says what it was
            // for. Measured on a copy of a real store, the two were about a
            // tenth of the sessions section. A block with no project of its
            // own lists several, and there the name is what tells them apart.
            let project = match own_project {
                Some(own) if own == session.project => String::new(),
                _ => format!(
                    "**{}** ",
                    crate::memory::normalize::one_line(&session.project)
                ),
            };
            context.push_str(&format!(
                "- {project}({}){summary}\n",
                crate::memory::normalize::to_the_minute(&session.last_activity),
            ));
        }
        context.push('\n');
    }
    if shown.prompts > 0 {
        context.push_str("### Recent User Prompts\n");
        for prompt in &prompts[..shown.prompts] {
            context.push_str(&format!(
                "- {}: {}\n",
                crate::memory::normalize::to_the_minute(&prompt.created_at),
                truncate(&prompt.content, PROMPT_LINE_CHARS)
            ));
        }
        context.push('\n');
    }
    if shown.pinned > 0 || pinned_omitted > 0 {
        context.push_str("### Pinned\n");
        for observation in &pinned[..shown.pinned] {
            context.push_str(&format!(
                "- #{} [{}] {}\n",
                observation.id,
                observation.kind,
                truncate(&observation.title, TITLE_CHARS)
            ));
            push_caveats(&mut context, caveats, &observation.sync_id);
        }
        // Said rather than swallowed: a pin is the most deliberate thing in the
        // store, and dropping one in silence is worse than the bytes it would
        // have cost. See `pinned_observations`. Pins the byte budget itself
        // drops are counted in the budget line below, not here.
        if pinned_omitted > 0 {
            context.push_str(&format!(
                "- ({pinned_omitted} more pinned, not shown - ask for them with mem_search)\n"
            ));
        }
        context.push('\n');
    }
    if !shown.listed.is_empty() {
        // An index, with no content under any of it, pinned memories included.
        //
        // The newest five used to open with their first three hundred
        // characters, and on a real store that was about eighteen hundred
        // tokens of recital before anybody had asked anything. Almost none of
        // it is read: what an agent needs up front is to know *what* is
        // remembered, and it pulls any of it with `mem_get_observation` once
        // one looks relevant. And a preview is a poor thing to answer from -
        // measured over 2417 memories of that store, 92% of the names, paths,
        // numbers and error strings fall past the first three hundred
        // characters and the median memory runs to 1969 - so a title that says
        // "fetch me" is more honest than a page that merely opens.
        //
        // The room goes to more titles instead; see `ContextSize`.
        context.push_str("### Also remembered — fetch with mem_get_observation\n");
        for &index in &shown.listed {
            let observation = &observations[index];
            context.push_str(&format!(
                "- #{} [{}] {}\n",
                observation.id,
                observation.kind,
                truncate(&observation.title, TITLE_CHARS)
            ));
            push_caveats(&mut context, caveats, &observation.sync_id);
        }
        context.push('\n');
    }
    let omitted_memories = observations.len() - shown.listed.len();
    let omitted_pinned = pinned.len() - shown.pinned;
    let omitted_prompts = prompts.len() - shown.prompts;
    let omitted_sessions = sessions.len() - shown.sessions;
    if omitted_memories + omitted_pinned + omitted_prompts + omitted_sessions > 0 {
        let mut parts: Vec<String> = Vec::new();
        if omitted_memories > 0 {
            parts.push(format!("{omitted_memories} memories"));
        }
        if omitted_pinned > 0 {
            parts.push(format!("{omitted_pinned} pinned"));
        }
        if omitted_prompts > 0 {
            parts.push(format!("{omitted_prompts} prompts"));
        }
        if omitted_sessions > 0 {
            parts.push(format!("{omitted_sessions} sessions"));
        }
        context.push_str(&format!(
            "- (cut to {budget_bytes} bytes; {} not shown - ask for memories with mem_search)\n",
            parts.join(", ")
        ));
    }
    context
}

/// Puts what the graph knows under the memory it is about.
///
/// Indented on its own line rather than appended, because the line above it has
/// already spent three hundred characters and a warning at the end of that is a
/// warning nobody reads. The title of the other memory comes along: "this was
/// overturned" without saying by what leaves the agent knowing only that it
/// cannot trust what it just read.
fn push_caveats(context: &mut String, caveats: &BTreeMap<String, Vec<Caveat>>, sync_id: &str) {
    for caveat in caveats.get(sync_id).into_iter().flatten() {
        context.push_str(&format!(
            "  ({} #{}: {})\n",
            caveat.verb.phrase(),
            caveat.other_id,
            truncate(&caveat.other_title, TITLE_CHARS)
        ));
    }
}

/// A title as a list prints it: one line, and short enough to sit on one.
///
/// Every surface that hands memories to an agent prints titles, and prints the
/// title of a *second* memory beside a caveat. This is the one rule they share,
/// so the block, the per-prompt hint and the handover cannot disagree about how
/// long a title may be; `normalize::one_line` is where the reason for folding
/// lives.
pub fn one_line_title(value: &str) -> String {
    truncate(value, TITLE_CHARS)
}

/// A preview, on one line, of at most `max_chars`.
///
/// Folded before it is cut, and that is the half that was missing. Everything
/// here goes into a markdown list an agent reads — one line per memory, per
/// prompt, per session, each starting with what it is about. A body is written
/// for a person: blank lines, bullets, headings. Dropped in whole it ended the
/// list at its first blank line and left a paragraph of body text standing at
/// the top level, directly above the *next* entry's line, with nothing saying
/// which of the two it belonged to.
///
/// Read off a real store, five memories came out as eighteen lines. The worse
/// case is a body carrying `- ` or `###` in the part that gets shown: those
/// arrive as an entry or a section heading that no memory owns, and the test
/// for this found it by having its own parsing cut short on one.
///
/// Folding also means the budget is spent on words rather than on newlines.
fn truncate(value: &str, max_chars: usize) -> String {
    crate::memory::normalize::truncate_words(value, max_chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembling_folds_summaries_before_it_truncates() {
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            crate::store::Store::open(crate::store::StoreConfig::new(temp.path().join("c.db")))
                .unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();

        // Twelve summaries saved after three real memories. Truncating first
        // would keep the twelve newest — all summaries — and the context would
        // say "there was a session" twelve times and carry nothing.
        let save = |store: &mut crate::store::Store, kind: &str, title: &str| {
            store
                .add_observation(crate::memory::model::AddObservation {
                    session_id: "s1".to_owned(),
                    kind: kind.to_owned(),
                    title: title.to_owned(),
                    content: format!("the body of {title}"),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        };
        for n in 0..3 {
            save(&mut store, "decision", &format!("A real memory {n}"));
        }
        for n in 0..12 {
            save(
                &mut store,
                "session_summary",
                &format!("Session summary {n}"),
            );
        }

        let context = assemble(&store, Some("leteo"), None, 5, usize::MAX).unwrap();
        for n in 0..3 {
            assert!(
                context.contains(&format!("A real memory {n}")),
                "the real memories survive a budget of five:
{context}"
            );
        }
    }

    fn observation(id: i64, title: &str) -> Observation {
        Observation {
            id,
            sync_id: format!("obs-{id}"),
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: title.to_owned(),
            content: "x".repeat(600),
            tool_name: None,
            project: Some("leteo".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            revision_count: 0,
            duplicate_count: 0,
            last_seen_at: None,
            review_after: None,
            prompt_sync_id: None,
            directory: None,
            pinned: false,
            created_at: "2026-07-30 10:00:00".to_owned(),
            updated_at: "2026-07-30 10:00:00".to_owned(),
            deleted_at: None,
        }
    }

    fn session(id: &str, summary: Option<&str>) -> SessionSummary {
        SessionSummary {
            id: id.to_owned(),
            project: "quarry".to_owned(),
            // Two dates a week apart, because a session that opened long ago
            // and saved something this morning is the case the rendering got
            // wrong, and a helper that gives both the same value cannot tell.
            started_at: "2026-07-30 10:00:00".to_owned(),
            last_activity: "2026-08-04 18:20:00".to_owned(),
            ended_at: None,
            summary: summary.map(str::to_owned),
            observation_count: 3,
        }
    }

    fn summary_of(session: &str, goal: &str) -> Observation {
        let mut observation = observation(99, "Session summary: quarry");
        observation.kind = "session_summary".to_owned();
        observation.session_id = session.to_owned();
        observation.content = format!(
            "## Goal
{goal}

## Discoveries
- something
"
        );
        observation
    }

    #[test]
    fn session_summaries_move_onto_their_sessions_instead_of_crowding_the_memories() {
        // On a live project thirty of the fifty most recent memories were
        // session summaries, so the context spent itself saying "there was a
        // session" while the decisions it was meant to carry were pushed out.
        // Meanwhile the sessions list showed names and dates and nothing else,
        // because the column those summaries also write to is often empty.
        let mut sessions = vec![session("s1", None), session("s2", Some("already here"))];
        let summaries = vec![
            summary_of("s1", "Ship the pagination work"),
            summary_of("s2", "This one must not overwrite"),
        ];
        let memories = vec![
            observation(1, "Chose SQLite"),
            observation(2, "Fixed the FK cascade"),
        ];

        fold_session_summaries(&mut sessions, summaries);

        assert_eq!(
            sessions[0].summary.as_deref(),
            Some("Ship the pagination work"),
            "the session gains what it was for, not the whole document"
        );
        assert_eq!(
            sessions[1].summary.as_deref(),
            Some("already here"),
            "a session that already had a summary keeps it"
        );

        let context = format_context(
            &sessions,
            &[],
            &[],
            0,
            &memories,
            &BTreeMap::new(),
            Frame {
                project: None,
                budget_bytes: usize::MAX,
            },
        );
        assert!(context.contains("Ship the pagination work"), "{context}");
        assert!(!context.contains("## Goal"), "{context}");
        assert!(
            !context.contains("Session summary"),
            "summaries must not appear as memories too: {context}"
        );
    }

    /// The dated line says when the session last did something.
    ///
    /// Both halves matter. Printing the start date puts a week-old date on the
    /// line the ordering just called the freshest, and the reader has no way to
    /// see it is out of order; the store's own listing came out 08-05, 07-28,
    /// 07-31, 07-27 with the second line the most recent of the four. The
    /// second assertion is what makes this test worth having: the start date
    /// must be gone, not merely joined.
    #[test]
    fn a_session_is_dated_by_its_last_activity_and_not_by_its_opening() {
        let context = format_context(
            &[session("s1", Some("the long one"))],
            &[],
            &[],
            0,
            &[],
            &BTreeMap::new(),
            Frame {
                project: None,
                budget_bytes: usize::MAX,
            },
        );

        assert!(
            context.contains("(2026-08-04 18:20)"),
            "the date must be the one the ordering uses: {context}"
        );
        assert!(
            !context.contains("2026-07-30 10:00"),
            "the opening date says the material is older than it is: {context}"
        );
    }

    /// Trimming a line's metadata must not change what the block selects.
    ///
    /// Compared against the store's own selection rather than against a
    /// snapshot of the old text: every memory, session and prompt the store
    /// hands the block has to be on a line of it, in the order the store gave
    /// them, and nothing the store did not hand it may appear.
    #[test]
    fn dropping_line_metadata_leaves_the_selection_alone() {
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            crate::store::Store::open(crate::store::StoreConfig::new(temp.path().join("c.db")))
                .unwrap();
        for (session, project) in [("a1", "alpha"), ("a2", "alpha"), ("b1", "beta")] {
            store.create_session(session, project, "C:/repo").unwrap();
            for n in 0..3 {
                store
                    .add_observation(crate::memory::model::AddObservation {
                        session_id: session.to_owned(),
                        kind: "decision".to_owned(),
                        title: format!("Memory {session}-{n}"),
                        content: format!("body {session}-{n}"),
                        tool_name: None,
                        project: Some(project.to_owned()),
                        scope: "project".to_owned(),
                        topic_key: None,
                        prompt_sync_id: None,
                    })
                    .unwrap();
            }
            store
                .add_prompt(crate::memory::model::AddPrompt {
                    session_id: session.to_owned(),
                    content: format!("prompt of {session}"),
                    project: Some(project.to_owned()),
                })
                .unwrap();
            store
                .end_session(session, Some(&format!("work of {session}")))
                .unwrap();
        }

        for scope in [Some("alpha"), None] {
            let block = assemble(&store, scope, None, 30, usize::MAX).unwrap();
            let sessions = store.recent_sessions(scope, Some(RECENT_SESSIONS)).unwrap();
            let lines = |heading: &str| -> Vec<String> {
                block
                    .split("\n\n")
                    .find(|section| section.starts_with(heading))
                    .map(|section| section.lines().skip(1).map(str::to_owned).collect())
                    .unwrap_or_default()
            };

            let listed = lines("### Recent Sessions");
            assert_eq!(listed.len(), sessions.len(), "{block}");
            for (line, session) in listed.iter().zip(&sessions) {
                let minute = crate::memory::normalize::to_the_minute(&session.last_activity);
                assert!(line.contains(&format!("({minute})")), "{line}");
                assert!(!line.contains("observations]"), "{line}");
                // Its own project is the block's heading, not a thing to say
                // forty times; any other project is the only way to tell the
                // lines apart.
                assert_eq!(
                    line.contains(&format!("**{}**", session.project)),
                    scope.is_none(),
                    "{line}"
                );
            }

            let prompts = store
                .recent_distinct_prompts(scope, Some(RECENT_PROMPTS))
                .unwrap();
            let listed = lines("### Recent User Prompts");
            assert_eq!(listed.len(), prompts.len(), "{block}");
            for (line, prompt) in listed.iter().zip(&prompts) {
                let minute = crate::memory::normalize::to_the_minute(&prompt.created_at);
                assert_eq!(line, &format!("- {minute}: {}", prompt.content));
            }

            let mut expected: Vec<i64> = store
                .pinned_observations(scope, None, 30)
                .unwrap()
                .0
                .iter()
                .map(|observation| observation.id)
                .collect();
            expected.extend(
                store
                    .recent_memories(scope, None, 30)
                    .unwrap()
                    .iter()
                    .map(|observation| observation.id),
            );
            let shown: Vec<i64> = block
                .lines()
                .filter_map(|line| line.strip_prefix("- #"))
                .filter_map(|rest| rest.split(' ').next()?.parse().ok())
                .collect();
            assert_eq!(shown, expected, "{block}");
        }
    }

    /// A smaller default names the same memories, fewer of them.
    ///
    /// Thirty is the first thirty of what fifty was, in the same order, and the
    /// pinned ones lead in both. Compared by id against the store's own
    /// selection at the larger count, so a change to the order or to what is
    /// skipped shows up here and not only in the count.
    #[test]
    fn the_default_size_names_the_first_of_what_a_larger_one_named() {
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            crate::store::Store::open(crate::store::StoreConfig::new(temp.path().join("c.db")))
                .unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let mut ids = Vec::new();
        for n in 0..70 {
            ids.push(
                store
                    .add_observation(crate::memory::model::AddObservation {
                        session_id: "s1".to_owned(),
                        kind: "decision".to_owned(),
                        title: format!("Memory {n}"),
                        content: format!("body {n}"),
                        tool_name: None,
                        project: Some("leteo".to_owned()),
                        scope: "project".to_owned(),
                        topic_key: None,
                        prompt_sync_id: None,
                    })
                    .unwrap()
                    .observation
                    .id,
            );
        }
        store.pin_observation(ids[3], None).unwrap();

        let named = |memories: usize| -> Vec<i64> {
            assemble(&store, Some("leteo"), None, memories, usize::MAX)
                .unwrap()
                .lines()
                .filter_map(|line| line.strip_prefix("- #"))
                .filter_map(|rest| rest.split(' ').next()?.parse().ok())
                .collect()
        };
        let default = crate::settings::ContextSize::default().memories();
        let larger = named(50);
        let smaller = named(default);

        assert_eq!(smaller.len(), default + 1, "thirty and the pin on top");
        assert_eq!(smaller[0], ids[3], "the pin leads");
        assert_eq!(smaller[..], larger[..smaller.len()]);
    }

    /// The two line budgets are two decisions, not one number twice.
    ///
    /// They were the same literal written out four times, and the number was
    /// right for one of them. A session summary has one population — median
    /// 182, p90 249, p99 304 over 338 of them on a real store — so cutting at
    /// 200 took the end off 37% of the lines whose whole job is to say what a
    /// session was for. A prompt has two: median 67, p90 414, because people
    /// paste. Cutting there is the point.
    #[test]
    fn a_session_line_and_a_prompt_line_are_cut_for_opposite_reasons() {
        // At compile time, because both sides are constants: editing either
        // one out of order fails the build rather than a run.
        const _: () = assert!(
            SESSION_LINE_CHARS > 304,
            "the session budget has to clear the p99 of a real store's summaries"
        );
        const _: () = assert!(
            PROMPT_LINE_CHARS < SESSION_LINE_CHARS,
            "a prompt is cut to keep a pasted file out, not to fit a sentence"
        );

        // A summary at the p90 arrives whole; a pasted prompt does not.
        let summary = "a".repeat(249);
        assert_eq!(truncate(&summary, SESSION_LINE_CHARS), summary);
        let pasted = "palabra ".repeat(200);
        let cut = truncate(&pasted, PROMPT_LINE_CHARS);
        assert!(
            cut.chars().count() <= PROMPT_LINE_CHARS,
            "{}",
            cut.chars().count()
        );
        assert!(cut.len() < pasted.len());
    }

    #[test]
    fn a_pinned_memory_is_listed_once_and_does_not_spend_the_budget_twice() {
        // Pinned memories are listed on their own, above the budget, because
        // deciding one matters should not cost the room the recent work needs.
        // They are then taken out of the recent list — without that they
        // appear in both, so the same memory is handed over twice, the budget
        // buys one fewer real memory for every pin, and the count reported
        // back is larger than what is on screen.
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            crate::store::Store::open(crate::store::StoreConfig::new(temp.path().join("c.db")))
                .unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let mut ids = Vec::new();
        for index in 0..6 {
            ids.push(
                store
                    .add_observation(crate::memory::model::AddObservation {
                        session_id: "s1".to_owned(),
                        kind: "decision".to_owned(),
                        title: format!("Memory {index}"),
                        content: format!("the body of memory {index}"),
                        tool_name: None,
                        project: Some("leteo".to_owned()),
                        scope: "project".to_owned(),
                        topic_key: None,
                        prompt_sync_id: None,
                    })
                    .unwrap()
                    .observation
                    .id,
            );
        }
        let pinned = ids[0];
        store.pin_observation(pinned, None).unwrap();

        let (context, listed) =
            assemble_counted(&store, Some("leteo"), None, 20, usize::MAX).unwrap();

        assert_eq!(
            context.matches(&format!("#{pinned} ")).count(),
            1,
            "the pinned memory was handed over twice:
{context}"
        );
        assert_eq!(listed, ids.len(), "each memory counted once");
    }

    #[test]
    fn no_memory_arrives_with_its_content_and_every_one_can_be_asked_for() {
        // The block is an index. Handing over memories in full would spend most
        // of a session's context reciting things nobody asked about, and an
        // agent that wants one of them can fetch it by the number on its line.
        let mut observations: Vec<Observation> = (1..=50)
            .map(|n| observation(n, &format!("Memory {n:02}")))
            .collect();
        for observation in &mut observations {
            observation.content = format!("BODY-OF-{}", observation.id).repeat(40);
        }
        let mut pinned = observation(51, "Pinned memory");
        pinned.pinned = true;
        pinned.content = "BODY-OF-51".repeat(40);
        let context = format_context(
            &[],
            &[],
            std::slice::from_ref(&pinned),
            0,
            &observations,
            &BTreeMap::new(),
            Frame {
                project: None,
                budget_bytes: usize::MAX,
            },
        );

        assert!(
            !context.contains("BODY-OF"),
            "a memory's content reached the opening block: {context}"
        );
        assert!(!context.contains("**"), "{context}");
        assert!(
            context.contains("- #51 [decision] Pinned memory"),
            "{context}"
        );
        assert!(context.contains("- #1 [decision] Memory 01"), "{context}");
        assert!(context.contains("- #50 [decision] Memory 50"), "{context}");
        assert!(
            context.contains("mem_get_observation"),
            "the index has to say how to read one of them: {context}"
        );
        // Fifty titles and a pin cost less than the twenty previews of three
        // hundred characters that used to be the head of this block.
        assert!(
            context.len() < 20 * 300,
            "fifty-one titles cost {} characters",
            context.len()
        );
    }

    #[test]
    fn a_session_opening_leaves_out_the_memories_a_later_one_overturned() {
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            crate::store::Store::open(crate::store::StoreConfig::new(temp.path().join("c.db")))
                .unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let save = |store: &mut crate::store::Store, title: &str| {
            store
                .add_observation(crate::memory::model::AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: title.to_owned(),
                    content: format!("the body of {title}"),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap()
                .observation
        };
        let older = save(&mut store, "We indent with tabs");
        let newer = save(&mut store, "We indent with spaces now");

        let before = assemble(&store, Some("leteo"), None, 20, usize::MAX).unwrap();
        assert!(!before.contains("superseded by"), "{before}");

        let relation = store
            .save_relation(crate::memory::model::SaveRelationParams {
                sync_id: crate::memory::normalize::sync_id("rel"),
                source_id: newer.sync_id.clone(),
                target_id: older.sync_id.clone(),
            })
            .unwrap();
        store
            .judge_relation(crate::memory::model::JudgeRelationParams {
                judgment_id: relation.sync_id,
                relation: crate::store::RELATION_SUPERSEDES.to_owned(),
                marked_by_actor: "agent".to_owned(),
                marked_by_kind: "agent".to_owned(),
                superseding_id: Some(newer.id),
                ..Default::default()
            })
            .unwrap();

        let after = assemble(&store, Some("leteo"), None, 20, usize::MAX).unwrap();

        assert!(
            !after.contains("We indent with tabs"),
            "a memory a later one overturned is not listed at all: {after}"
        );
        assert!(
            !after.contains("superseded by"),
            "and the context carries no caveat for a memory it did not list: {after}"
        );
        assert!(
            after.contains("We indent with spaces now"),
            "the memory that overtook it is the one that stands: {after}"
        );

        // It is hidden from the listing, not from a direct read: the caveat is
        // still there for the id an agent may already hold.
        assert_eq!(
            store
                .caveats_for(std::slice::from_ref(&older.sync_id))
                .unwrap()
                .get(&older.sync_id)
                .map(Vec::len),
            Some(1),
            "a fetch by id still says it was overturned"
        );
    }

    #[test]
    fn every_memory_the_context_names_can_be_asked_for_by_number() {
        // The entries that carried a preview were once the only ones without an
        // id - so the ones a reader was most likely to want more of were the
        // ones it could not ask for. An A/B run caught it: the answer sat at
        // character 337 of a 2177-character memory and the model answered
        // without it. Every entry is a title now, and each still has its number.
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            crate::store::Store::open(crate::store::StoreConfig::new(temp.path().join("c.db")))
                .unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        let mut ids = Vec::new();
        for index in 0..8 {
            let saved = store
                .add_observation(crate::memory::model::AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: format!("Decision {index}"),
                    content: "x".repeat(600),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap()
                .observation;
            ids.push(saved.id);
        }
        store.pin_observation(ids[0], None).unwrap();

        let context = assemble(&store, Some("leteo"), None, 20, usize::MAX).unwrap();

        for id in &ids {
            assert!(
                context.contains(&format!("#{id} ")),
                "memory {id} is named without a number to fetch it by:
{context}"
            );
        }
        assert!(
            !context.contains("xxxx"),
            "no body reaches the block, so nothing is cut in it: {context}"
        );
    }

    /// The skill promises agents a block of titles, and the code renders one.
    ///
    /// The promise is load-bearing rather than decorative: the skill tells
    /// agents to assume the answer is not in what they were handed and to
    /// fetch the whole memory by `#id`. It used to say the newest few open with
    /// their first three hundred characters; the block carries no content now,
    /// and a skill still describing previews would have agents look for text
    /// that is not there and trust the little of it that is.
    #[test]
    fn the_skill_promises_titles_and_the_block_carries_nothing_else() {
        let mut memory = observation(1, "A title");
        memory.content = "a body that must not appear".to_owned();
        let context = format_context(
            &[],
            &[],
            &[],
            0,
            std::slice::from_ref(&memory),
            &BTreeMap::new(),
            Frame {
                project: None,
                budget_bytes: usize::MAX,
            },
        );
        assert!(
            !context.contains("a body that must not appear"),
            "{context}"
        );

        for skill in [
            "plugin/claude-code/skills/memory/SKILL.md",
            "plugin/codex/skills/memory/SKILL.md",
            "plugin/zcode/skills/memory/SKILL.md",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(skill);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(
                text.contains("is a title alone"),
                "{skill} has to tell agents every entry of the opening block is a title alone"
            );
            assert!(
                !text.contains("three hundred"),
                "{skill} still promises a preview the block no longer carries"
            );
        }
    }

    #[test]
    fn a_context_limit_from_outside_cannot_overflow_the_fetch() {
        // `memories` arrives unbounded from `leteo context --limit` and from
        // `mem_context`'s `limit`, and the fetch asks for four times it.
        //
        // A plain multiplication panicked the binary on a large one — verified
        // against the real CLI, which printed "attempt to multiply with
        // overflow" and died. A release build does not panic; it wraps, and
        // hands back a handful of memories to somebody who asked for every one
        // of them, which is the quieter half of the same bug.
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            crate::store::Store::open(crate::store::StoreConfig::new(temp.path().join("o.db")))
                .unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: "one memory to find".to_owned(),
                content: "a body".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();

        for asked in [usize::MAX, usize::MAX / 2, usize::MAX / 4 + 1] {
            let (context, listed) =
                assemble_counted(&store, Some("leteo"), None, asked, usize::MAX)
                    .expect("a large limit is a small answer, not a crash");
            assert!(
                context.contains("Memory from Previous Sessions"),
                "{asked} produced no context at all"
            );
            assert!(listed <= asked);
        }
    }
    #[test]
    fn a_body_cannot_put_structure_into_the_block() {
        // The context is a list an agent reads: one line per memory, each
        // starting with its id. A body is written for a person - blank lines,
        // bullets, headings - and when a cut of it went into the list, one
        // memory became a list item, a blank line that ends the list, and a
        // paragraph hanging at the top level above the *next* memory's line.
        // Read off a real store, five memories rendered as eighteen physical
        // lines. No body reaches the block now, so what is asserted is that
        // none of its structure does either.
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            crate::store::Store::open(crate::store::StoreConfig::new(temp.path().join("p.db")))
                .unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "bugfix".to_owned(),
                title: "The floor was compared the wrong way round".to_owned(),
                content: "**Found by measuring.**

### What broke

- the good candidates scored past the floor
- so every one of them was thrown away for scoring too well

And the tests could not see it, because a small store scores near zero."
                    .to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();

        let context = assemble(&store, Some("leteo"), None, 10, usize::MAX).unwrap();
        let entry = context
            .lines()
            .find(|line| line.starts_with("- #1 "))
            .expect("the memory is listed");
        assert_eq!(
            entry, "- #1 [bugfix] The floor was compared the wrong way round",
            "the line is the id, the kind and the title"
        );
        // And nothing the body carried is now standing on its own. A heading
        // from inside a memory ends the section for whoever reads it — this
        // test found that by having its own parsing cut short on one.
        for injected in [
            "
### What broke",
            "
- the good candidates",
        ] {
            assert!(
                !context.contains(injected),
                "a memory's own markdown reached the context as structure: {injected:?}"
            );
        }
    }
    #[test]
    fn the_pinned_section_and_the_index_cut_their_title_like_every_other_line() {
        // The block promises that every line of the block is cut at a bound
        // somebody measured, and names the title bound first. Of the places in
        // this file that print a title, two kept that promise — the `Also
        // remembered` index and the title of the second memory named beside a
        // caveat — and the `Pinned` section and the section of newest memories
        // did not. They passed the title through `one_line`, which folds
        // whitespace and cuts nothing, so the only ceiling left on it was
        // `max_observation_length`: fifty thousand bytes, the same cap a
        // memory's body gets. The newest-memories section is gone, its
        // previews with it; the pinned line is the one left to drive.
        //
        // The three sites outside this file - the per-prompt hint, the title
        // of the other memory in a hint caveat, and the verdict handover, all
        // in `src/hooks/context.rs` - were unbounded the same way and now go
        // through `one_line_title`, with a test each in `src/hooks/tests.rs`.
        // `one_line_title` is the one rule for how long a printed title may be.
        //
        // Nothing had fallen in when this was fixed. Measured over a copy of a
        // real store, the two sections rendered 89 title lines across 27
        // projects and the longest was 132 characters — but 93 of that store's
        // 4,756 titles were already past the bound and would have been cut had
        // they landed in either window.
        //
        // Both sections are driven here rather than one, because the pair is
        // the defect: they were written together and skipped together.
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            crate::store::Store::open(crate::store::StoreConfig::new(temp.path().join("p.db")))
                .unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        // Distinct text per memory, not the same one twice: an identical title
        // and body normalise to the same hash and the store keeps one, which
        // left this driving a single section and calling it both.
        for nth in ["first", "second"] {
            let long = format!("{} {nth}", "sostenido ".repeat(60));
            assert!(
                long.chars().count() > TITLE_CHARS,
                "the title this drives with has to be past the bound to test it"
            );
            store
                .add_observation(crate::memory::model::AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: long,
                    content: format!("a body, the {nth} one"),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
        // One of the two pinned, so the same assembly renders a `### Pinned`
        // line and an `### Also remembered` line and both are checked.
        store.pin_observation(1, None).unwrap();

        let context = assemble(&store, Some("leteo"), None, 10, usize::MAX).unwrap();
        let titled: Vec<&str> = context
            .lines()
            .filter(|line| line.starts_with("- #"))
            .collect();
        assert_eq!(
            titled.len(),
            2,
            "expected one pinned line and one remembered line: {context}"
        );
        for line in titled {
            let title = line.split_once("] ").expect("the title follows the kind").1;
            assert!(
                title.chars().count() <= TITLE_CHARS,
                "a title reached the block uncut at {} characters: {line}",
                title.chars().count()
            );
        }
    }

    fn prompt(n: usize) -> Prompt {
        Prompt {
            id: n as i64,
            sync_id: format!("prompt-{n}"),
            session_id: "s1".to_owned(),
            content: format!("prompt {n} {}", "a long request ".repeat(40)),
            project: "quarry".to_owned(),
            created_at: format!("2026-08-04 10:{:02}:00", 59 - n),
        }
    }

    /// Five dated sessions, newest first, each named `session N` in its summary.
    fn dated_sessions(count: usize) -> Vec<SessionSummary> {
        (0..count)
            .map(|n| {
                let mut session = session(
                    &format!("s{n}"),
                    Some(&format!("session {n} {}", "did a lot of work ".repeat(10))),
                );
                session.last_activity = format!("2026-08-0{} 18:20:00", 5 - n);
                session
            })
            .collect()
    }

    fn thirty_memories() -> Vec<Observation> {
        (0..30)
            .map(|n| {
                observation(
                    100 - n,
                    &format!(
                        "A long and descriptive title that costs real bytes, memory number {n}"
                    ),
                )
            })
            .collect()
    }

    fn render(
        sessions: &[SessionSummary],
        prompts: &[Prompt],
        memories: &[Observation],
        budget_bytes: usize,
    ) -> (String, usize) {
        format_context_counted(
            sessions,
            prompts,
            &[],
            0,
            memories,
            &BTreeMap::new(),
            Frame {
                project: Some("quarry"),
                budget_bytes,
            },
        )
    }

    /// The sections that frame the memories give way before the memories do.
    ///
    /// Measured on a copy of a real store, the four largest projects' blocks
    /// were all cut at 4,000 bytes and left out 15, 10, 14 and 10 of thirty
    /// memories while prompts took 1,301 bytes of one of them. The fixture is
    /// oversized on purpose: with every section whole it is far over the budget.
    #[test]
    fn prompts_are_dropped_before_any_memory_title() {
        let sessions = dated_sessions(5);
        let prompts: Vec<Prompt> = (0..10).map(prompt).collect();
        let memories = thirty_memories();

        let (everything, _) = render(&sessions, &prompts, &memories, usize::MAX);
        assert!(everything.len() > 4_000, "{}", everything.len());

        let (context, named) = render(&sessions, &prompts, &memories, 4_000);
        assert!(context.len() <= 4_000, "{} bytes", context.len());
        for n in 0..30 {
            assert!(
                context.contains(&format!("memory number {n}\n")),
                "memory {n} was cut while prompts were still available: {context}"
            );
        }
        assert_eq!(named, 30, "{context}");
        assert!(
            !context.contains("Recent User Prompts"),
            "prompts should have given way first: {context}"
        );
        assert!(
            !context.contains("memories not shown") && !context.contains("30 memories"),
            "no memory was cut, so none may be reported as cut: {context}"
        );
        assert!(
            context.contains("10 prompts not shown"),
            "what was dropped has to be counted: {context}"
        );
    }

    /// Sessions give way after prompts and before memories, oldest first, down
    /// to one.
    #[test]
    fn sessions_are_dropped_oldest_first_down_to_one_before_any_memory_title() {
        let sessions = dated_sessions(5);
        let memories = thirty_memories();

        let (one_session, _) = render(&sessions[..1], &[], &memories, usize::MAX);
        // Room for the memories, one session and the line that counts the
        // cut, and not for a second session.
        let budget = one_session.len() + 120;
        let (context, named) = render(&sessions, &[], &memories, budget);

        assert!(context.len() <= budget, "{} > {budget}", context.len());
        assert_eq!(named, 30, "{context}");
        assert!(context.contains("session 0 "), "newest kept: {context}");
        assert!(!context.contains("session 1 "), "older gone: {context}");
        assert!(!context.contains("session 4 "), "oldest gone: {context}");
        assert!(
            context.contains("4 sessions not shown"),
            "what was dropped has to be counted: {context}"
        );
    }

    /// Memories are cut only when nothing else is left to cut, and the cut is
    /// still counted.
    #[test]
    fn memories_are_cut_last_and_the_cut_is_counted() {
        let sessions = dated_sessions(5);
        let prompts: Vec<Prompt> = (0..10).map(prompt).collect();
        let memories = thirty_memories();

        let (context, named) = render(&sessions, &prompts, &memories, 1_200);
        assert!(context.len() <= 1_200, "{} bytes", context.len());
        assert!(named < 30, "{context}");
        assert!(
            context.contains("session 0 "),
            "one session stays: {context}"
        );
        assert!(!context.contains("Recent User Prompts"), "{context}");
        assert!(
            context.contains(&format!("{} memories", 30 - named)),
            "the memories that were cut have to be counted: {context}"
        );
        assert!(
            context.contains("memory number 0\n"),
            "the newest title is kept: {context}"
        );
    }

    /// A session with nothing to say is a date and nothing else.
    #[test]
    fn a_session_without_summary_text_is_not_listed() {
        let sessions = vec![
            session("s0", Some("the one with words")),
            session("s1", None),
            session("s2", Some("   ")),
        ];
        let (context, _) = render(&sessions, &[], &thirty_memories(), usize::MAX);
        assert_eq!(
            context.matches("- (2026-08-04 18:20)").count(),
            1,
            "only the session with a summary has a line: {context}"
        );
        assert!(context.contains("the one with words"), "{context}");
        assert!(
            !context.contains("- (2026-08-04 18:20)\n"),
            "a bare date line was printed: {context}"
        );

        let (only_bare, named) = render(&sessions[1..], &[], &[], usize::MAX);
        assert_eq!(only_bare, "", "nothing to say means no block");
        assert_eq!(named, 0);
    }

    /// A block is never larger than the budget, and says what it left out.
    ///
    /// The count ceiling and the byte budget are different bounds: thirty
    /// memories inside the count can still be more bytes than the block is
    /// allowed, and the block is paid on every session and after every
    /// compaction. The fixture is oversized on purpose — a store whose rows are
    /// small enough to fit would pass this without watching anything.
    #[test]
    fn a_block_is_never_larger_than_the_budget_and_says_what_it_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let mut store =
            crate::store::Store::open(crate::store::StoreConfig::new(temp.path().join("b.db")))
                .unwrap();
        store.create_session("s1", "leteo", "C:/repo").unwrap();
        for index in 0..30 {
            store
                .add_observation(crate::memory::model::AddObservation {
                    session_id: "s1".to_owned(),
                    kind: "decision".to_owned(),
                    title: format!("Memory number {index} with a title long enough to cost bytes"),
                    content: "cuerpo ".repeat(200),
                    tool_name: None,
                    project: Some("leteo".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }

        let budget = 2_000;
        let (context, named) = assemble_counted(&store, Some("leteo"), None, 30, budget).unwrap();
        assert!(
            context.len() <= budget,
            "the block is {} bytes against a {budget}-byte budget: {context}",
            context.len()
        );
        // The count and the block come from the same render. The hook that
        // reports the number to a person reads this one, so it has to be what
        // the block names and not what it was asked for.
        let lines = context
            .lines()
            .filter(|line| line.starts_with("- #"))
            .count();
        assert_eq!(
            named, lines,
            "the count has to be what the block names, not what it dropped: {context}"
        );
        assert!(
            named < 30,
            "the oversized fixture has to make the count smaller than what was found: {context}"
        );
        assert!(
            context.contains(&format!("cut to {budget} bytes")),
            "the cut has to be counted: {context}"
        );
        assert!(
            context.contains("not shown"),
            "the cut has to name what is missing: {context}"
        );
        assert!(
            context.contains("Memory number 29"),
            "the newest memory is the one the block keeps: {context}"
        );
        assert!(
            !context.contains("Memory number 0 "),
            "the oldest memory is the one the budget drops first: {context}"
        );
    }
}
