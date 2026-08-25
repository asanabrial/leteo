//! The memory blob an agent is handed when a session starts.
//!
//! Shared by `leteo context` and by the session-start and compaction hooks, so
//! what an agent reads at the top of a conversation is written in one place
//! rather than once per surface that hands it over.

use std::collections::BTreeMap;

use crate::memory::model::{Caveat, Observation, Prompt, SessionSummary};

/// How many of the most recent memories are handed over with their content.
///
/// The rest arrive as titles. Five is about one screenful of real detail:
/// enough to carry on from where the last session stopped, and not so much that
/// a conversation opens with a wall of text nobody asked for.
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

/// How much of a memory the previewed entries open with.
///
/// Not the same cut as the MCP tools make: `PREVIEW_BYTES` is 400 there,
/// because a tool result is fetched deliberately and pays for itself, while
/// this blob is spent on every session whether or not it is read. The two are
/// allowed to differ; what is not allowed is for either to drift silently.
///
/// The skill promises this number to agents in words — it tells them to assume
/// the answer is past the cut and to fetch by `#id` — so a change here is a
/// change to a published promise. `the_skill_promises_the_preview_length_the_code_cuts_at`
/// holds the two together.
pub(crate) const CONTEXT_PREVIEW_CHARS: usize = 300;

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
) -> Result<String, crate::StoreError> {
    assemble_counted(store, project, scope, memories).map(|(context, _)| context)
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
) -> Result<(String, usize), crate::StoreError> {
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
    let listed = pinned.len() + observations.len();
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
    Ok((
        format_context(
            &sessions,
            &prompts,
            &pinned,
            pinned_omitted,
            &observations,
            &caveats,
        ),
        listed,
    ))
}

/// How many memories a context carries when the caller does not say.
///
/// From the setting, because three surfaces build this same context and two of
/// them asked. `leteo context` used a constant twenty while the session-start
/// hook and `mem_context` both read `context_size`, whose default is fifty —
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

/// How many sessions and prompts the opening context names.
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

/// The ids of the sessions about to be listed.
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
) -> String {
    if sessions.is_empty() && prompts.is_empty() && pinned.is_empty() && observations.is_empty() {
        return String::new();
    }

    let mut context = String::from("## Memory from Previous Sessions\n\n");
    if !sessions.is_empty() {
        context.push_str("### Recent Sessions\n");
        for session in sessions {
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
            context.push_str(&format!(
                "- **{}** ({}){} [{} observations]\n",
                crate::memory::normalize::one_line(&session.project),
                session.last_activity,
                summary,
                session.observation_count
            ));
        }
        context.push('\n');
    }
    if !prompts.is_empty() {
        context.push_str("### Recent User Prompts\n");
        for prompt in prompts {
            context.push_str(&format!(
                "- {}: {}\n",
                prompt.created_at,
                truncate(&prompt.content, PROMPT_LINE_CHARS)
            ));
        }
        context.push('\n');
    }
    if !pinned.is_empty() {
        context.push_str("### Pinned\n");
        for observation in pinned {
            context.push_str(&format!(
                "- #{} [{}] **{}**: {}\n",
                observation.id,
                observation.kind,
                truncate(&observation.title, TITLE_CHARS),
                truncate(&observation.content, CONTEXT_PREVIEW_CHARS)
            ));
            push_caveats(&mut context, caveats, &observation.sync_id);
        }
        // Said rather than swallowed: a pin is the most deliberate thing in the
        // store, and dropping one in silence is worse than the bytes it would
        // have cost. See `pinned_observations`.
        if pinned_omitted > 0 {
            context.push_str(&format!(
                "- ({pinned_omitted} more pinned, not shown - ask for them with mem_search)\n"
            ));
        }
        context.push('\n');
    }
    if !observations.is_empty() {
        // The newest few in full, everything behind them as an index.
        //
        // Three hundred characters per memory is the shape this was inherited
        // with, and on a real store it spent about eighteen hundred tokens
        // reciting twenty-nine memories in full before anybody had asked a
        // question. Almost none of that content gets read: what an agent needs
        // up front is to know *what* is remembered, and it can pull any of it
        // with `mem_get_observation` the moment one looks relevant.
        //
        // So the recent handful keep their content — those are the ones the
        // work is likely continuing from — and the rest become one line each.
        // That buys far more memories for far fewer tokens, which is the whole
        // point: an index of fifty beats a recital of twenty.
        let split = DETAILED.min(observations.len());
        let (detailed, listed) = observations.split_at(split);
        // The heading says these are previews, because they do not look like
        // previews. Measured over 2417 memories of this store: 92% of the
        // names, paths, numbers and error strings — the part the skill asks to
        // spend freely on — falls past the first three hundred characters, and
        // the median memory runs to 1969. An agent that answers from what is
        // here is answering from the opening of a page.
        context.push_str(
            "### Recent Observations — previews; read one in full with mem_get_observation\n",
        );
        for observation in detailed {
            context.push_str(&format!(
                "- #{} [{}] **{}**: {}\n",
                observation.id,
                observation.kind,
                truncate(&observation.title, TITLE_CHARS),
                truncate(&observation.content, CONTEXT_PREVIEW_CHARS)
            ));
            push_caveats(&mut context, caveats, &observation.sync_id);
        }
        context.push('\n');
        if !listed.is_empty() {
            context.push_str("### Also remembered — fetch with mem_get_observation\n");
            for observation in listed {
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
/// A title as a list prints it: one line, and short enough to sit on one.
///
/// The two surfaces that hand memories to an agent both print titles, and both
/// print the title of a *second* memory beside a caveat. This is the shape
/// they agree on; `normalize::one_line` is where the reason lives.
pub fn one_line_title(value: &str) -> String {
    truncate(value, TITLE_CHARS)
}

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

        let context = assemble(&store, Some("leteo"), None, 5).unwrap();
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
            // Three hundred characters, which is what the formatter keeps.
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

        // And the goal is what shows, not the heading above it.
        let context = format_context(&sessions, &[], &[], 0, &memories, &BTreeMap::new());
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
        );

        assert!(
            context.contains("2026-08-04 18:20:00"),
            "the date must be the one the ordering uses: {context}"
        );
        assert!(
            !context.contains("2026-07-30 10:00:00"),
            "the opening date says the material is older than it is: {context}"
        );
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
        store.pin_observation(pinned).unwrap();

        let (context, listed) = assemble_counted(&store, Some("leteo"), None, 20).unwrap();

        assert_eq!(
            context.matches(&format!("#{pinned} ")).count(),
            1,
            "the pinned memory was handed over twice:
{context}"
        );
        assert_eq!(listed, ids.len(), "each memory counted once");
    }

    #[test]
    fn only_the_newest_memories_arrive_with_their_content() {
        // The rest are titles. Handing over fifty memories in full would spend
        // most of a session's context reciting things nobody asked about, and
        // an agent that wants one of them can fetch it.
        let observations: Vec<Observation> = (1..=50)
            .map(|n| observation(n, &format!("Memory {n:02}")))
            .collect();
        let context = format_context(&[], &[], &[], 0, &observations, &BTreeMap::new());

        assert_eq!(
            context.matches("[decision] **").count(),
            DETAILED,
            "only the newest few carry content"
        );
        // Both shapes carry the number. The ones with content are the ones
        // somebody is most likely to want more of, and they were the only ones
        // that could not be asked for.
        assert!(
            context.contains("- #1 [decision] **Memory 01**:"),
            "{context}"
        );
        assert!(context.contains("- #50 [decision] Memory 50"), "{context}");
        assert!(
            context.contains("mem_get_observation"),
            "the index has to say how to read one of them: {context}"
        );

        // And the whole thing stays inside a budget. Fifty memories as an index
        // has to cost less than twenty did as a recital — that trade is the
        // entire point, and a later change that quietly undoes it would give
        // back the tokens without giving back the reach.
        let recital = 20 * CONTEXT_PREVIEW_CHARS;
        assert!(
            context.len() < recital,
            "fifty indexed memories cost {} characters; twenty recited cost {recital}",
            context.len()
        );
    }

    #[test]
    fn a_session_opening_marks_the_memories_a_later_one_overturned() {
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

        let before = assemble(&store, Some("leteo"), None, 20).unwrap();
        assert!(!before.contains("superseded by"), "{before}");

        let relation = store
            .save_relation(crate::memory::model::SaveRelationParams {
                sync_id: crate::memory::normalize::sync_id("rel"),
                source_id: newer.sync_id.clone(),
                target_id: older.sync_id,
            })
            .unwrap();
        store
            .judge_relation(crate::memory::model::JudgeRelationParams {
                judgment_id: relation.sync_id,
                relation: crate::store::RELATION_SUPERSEDES.to_owned(),
                marked_by_actor: "agent".to_owned(),
                marked_by_kind: "agent".to_owned(),
                ..Default::default()
            })
            .unwrap();

        let after = assemble(&store, Some("leteo"), None, 20).unwrap();

        assert!(
            after.contains(&format!(
                "(superseded by #{}: We indent with spaces now)",
                newer.id
            )),
            "the opening context hands over fifty memories and has to say which of them no longer \
             hold: {after}"
        );
        assert_eq!(
            after.matches("superseded by").count(),
            1,
            "only the memory that was overturned, not the one that did it: {after}"
        );
    }

    #[test]
    fn every_memory_the_context_names_can_be_asked_for_by_number() {
        // The entries shown with their content are cut at three hundred
        // characters, and they were the only ones without an id — so the ones
        // a reader is most likely to want more of were the ones it could not
        // ask for. An A/B run caught it: the answer sat at character 337 of a
        // 2177-character memory and the model answered without it.
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
        store.pin_observation(ids[0]).unwrap();

        let context = assemble(&store, Some("leteo"), None, 20).unwrap();

        for id in &ids {
            assert!(
                context.contains(&format!("#{id} ")),
                "memory {id} is named without a number to fetch it by:
{context}"
            );
        }
        assert!(
            context.contains("..."),
            "a body was cut, and the cut has to be visible"
        );
    }

    /// The skill promises agents this preview length in words, and the code
    /// cuts at it.
    ///
    /// The promise is load-bearing rather than decorative: the skill tells
    /// agents to assume the answer lies past the cut and to fetch the whole
    /// memory by `#id` instead of answering from what they can already see.
    /// Changing the constant without the sentence leaves that advice calibrated
    /// to a number the code stopped using, and nothing else would notice.
    #[test]
    fn the_skill_promises_the_preview_length_the_code_cuts_at() {
        let spelled = match CONTEXT_PREVIEW_CHARS {
            200 => "two hundred",
            300 => "three hundred",
            400 => "four hundred",
            500 => "five hundred",
            other => panic!(
                "the context preview is now {other} characters; spell it out here and in both SKILL.md files"
            ),
        };
        for skill in [
            "plugin/claude-code/skills/memory/SKILL.md",
            "plugin/codex/skills/memory/SKILL.md",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(skill);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(
                text.contains(spelled),
                "{skill} has to tell agents the context opens with the first {spelled} characters"
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
            let (context, listed) = assemble_counted(&store, Some("leteo"), None, asked)
                .expect("a large limit is a small answer, not a crash");
            assert!(
                context.contains("Memory from Previous Sessions"),
                "{asked} produced no context at all"
            );
            assert!(listed <= asked);
        }
    }
    #[test]
    fn a_preview_is_one_line_however_many_the_memory_has() {
        // The context is a list an agent reads: one line per memory, each
        // starting with its id. A preview is three hundred characters cut out
        // of a body written for a person — blank lines, bullets, headings —
        // and dropped in whole, so one memory became a list item, a blank line
        // that ends the list, and a paragraph of body text hanging at the top
        // level directly above the *next* memory's line.
        //
        // Read off a real store: five memories rendered as eighteen physical
        // lines. The harm is not the shape but the attribution — an agent has
        // no way to tell that stray paragraph belongs to the entry above it
        // rather than the one below, and a body carrying `- ` or `###` in its
        // first three hundred characters injects an entry or a section heading
        // that no memory owns.
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

        let context = assemble(&store, Some("leteo"), None, 10).unwrap();
        let entry = context
            .lines()
            .find(|line| line.starts_with("- #1 "))
            .expect("the memory is listed");
        assert!(
            entry.contains("thrown away for scoring too well"),
            "the preview is cut across lines: {entry}"
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
    fn the_two_sections_that_carry_a_preview_cut_their_title_like_every_other_line() {
        // `hooks.md` §5 promises that every line of the block is cut at a bound
        // somebody measured, and names the title bound first. Of the four places
        // in this file that print a title, two kept that promise — the `Also
        // remembered` index and the title of the second memory named beside a
        // caveat — and the two that also print a *content* preview did not. They
        // passed the title through `one_line`, which folds whitespace and cuts
        // nothing, so the only ceiling left on it was `max_observation_length`:
        // fifty thousand bytes, the same cap a memory's body gets.
        //
        // The census is worth stating exactly, because an earlier version of
        // this comment counted `one_line_title` as one of the sites that kept
        // the promise and it prints nothing: no production path calls it, only
        // `a_caveat_in_the_hint_prints_one_short_line`, which is a guard for
        // the hint that never runs the hint. Two blind reviewers caught that
        // independently. Three genuine sites are still unbounded and are not in
        // this file — `src/hooks/context.rs:105`, `:112` and `:310` — so a
        // reader auditing "every place a title is printed" should start there
        // rather than believe this block is the whole surface.
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
        // line and a `### Recent Observations` line and both are checked.
        store.pin_observation(1).unwrap();

        let context = assemble(&store, Some("leteo"), None, 10).unwrap();
        let previewed: Vec<&str> = context
            .lines()
            .filter(|line| line.starts_with("- #") && line.contains("**"))
            .collect();
        assert_eq!(
            previewed.len(),
            2,
            "expected one pinned line and one detailed line: {context}"
        );
        for line in previewed {
            let title = line
                .split("**")
                .nth(1)
                .expect("the title is the part between the asterisks");
            assert!(
                title.chars().count() <= TITLE_CHARS,
                "a title reached the block uncut at {} characters: {line}",
                title.chars().count()
            );
        }
    }
}
