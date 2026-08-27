//! What the agent is handed back.
//!
//! Three questions with three different answers: what a prompt should have in
//! front of it, what a session opening or a compaction should be rebuilt from,
//! and how much this project holds. Each returns the text *and* the count, so
//! that what the agent reads and what the person is told cannot disagree.

use crate::Store;

use super::HookOutcome;

/// How many are named. Beyond three it stops being a hint and becomes a list.
///
/// Visible to [`super::nudge`], which sizes the list of what this conversation
/// has already been handed from it: at most this many new memories a prompt.
pub(super) const RECALL_LIMIT: usize = 3;

/// Memories worth putting in front of this prompt, or nothing at all.
///
/// Nothing at all about one prompt in five, by design and by measurement.
/// Against 277 prompts on a real store, labelled without leaking the future —
/// the memories saved earlier in the same session, which existed when the hint
/// would have spoken — it speaks on 81% and names something from that session
/// on 22%, better than one time in four when it speaks at all.
///
/// The label is a stand-in and it undercounts: a session touches several
/// subjects, and a hint that brings back something genuinely useful from a
/// different session is scored here as a miss. What it is good for is
/// comparing two ways of choosing, since none of them can see the session or
/// the clock.
///
/// What the choosing is: a relevance floor relative to the median of the
/// candidates, and it is not the same floor on both sides. The session opened
/// with an index of the project's most recent memories, so naming one of those
/// again only repeats what the agent is already holding, while naming an older
/// one is the only way it hears of that memory at all — see
/// `Store::worth_naming`. That asymmetry is worth ten points of accuracy over
/// one floor for everything.
///
/// Most of what is still missed is missed by full-text search rather than by
/// the threshold: a third of the failures are a question asked in Spanish
/// against a memory written in English, and no margin fixes that.
///
/// Which is why the wording hedges rather than announces. A hint that is wrong
/// most of the time has to read like a hint; an agent told "here is the answer"
/// and handed the wrong one is worse off than an agent told nothing. Returns
/// the context for the agent and how many memories it names, because the person
/// watching is told the count and the two must not disagree.
pub(super) fn prompt_recall(
    store: &Store,
    prompt: &str,
    project: &str,
    already_shown: &[i64],
) -> Option<(String, usize, Vec<i64>)> {
    let mut matches = store.prompt_matches(prompt, project, RECALL_LIMIT).ok()?;
    // What the graph knows about the three about to be named. Leteo pays a
    // language model to judge these pairs and then read nothing back; a memory
    // a later one has overturned used to look exactly like one that still
    // stands. Failing to reach the graph costs the annotation, not the hint.
    let sync_ids: Vec<String> = matches
        .iter()
        .map(|observation| observation.sync_id.clone())
        .collect();
    let caveats = store.caveats_for(&sync_ids).unwrap_or_default();

    // Not what this conversation has already been handed — unless something is
    // said against it.
    //
    // A conversation stays about the same thing for a while, so the same
    // memories keep winning: over six real sessions, 134 of 273 memories named
    // here were named again having already been named in that same session. The
    // agent has them; saying them twice spends the room a new one would have
    // taken and teaches whoever reads the hint that it repeats itself.
    //
    // The exception is not a hedge. A memory can be overturned *after* it was
    // named — somebody judges the pair mid-conversation — and silence then
    // leaves the agent holding the version it was handed before anybody said
    // anything against it. Naming it again is the only way the caveat arrives,
    // and that is the whole reason caveats are carried here at all.
    //
    // Dropped rather than backfilled from further down the ranking. What is
    // below the third result did not pass the relevance test on its own merits,
    // and filling the gap with it would trade a repeat nobody needed for a
    // stranger nobody asked for.
    matches.retain(|observation| {
        !already_shown.contains(&observation.id)
            || caveats
                .get(&observation.sync_id)
                .is_some_and(|said| !said.is_empty())
    });
    if matches.is_empty() {
        return None;
    }
    let shown: Vec<i64> = matches.iter().map(|observation| observation.id).collect();

    let mut recall = String::from(
        "Leteo may have something on this. Read one with mem_get_observation \
         before assuming, and ignore any that do not fit.\n",
    );
    for observation in &matches {
        recall.push_str(&format!(
            "- #{} [{}] {}\n",
            observation.id,
            observation.kind,
            crate::memory::normalize::one_line(&observation.title)
        ));
        for caveat in caveats.get(&observation.sync_id).into_iter().flatten() {
            recall.push_str(&format!(
                "  ({} #{}: {})\n",
                caveat.verb.phrase(),
                caveat.other_id,
                caveat.other_title
            ));
        }
    }
    Some((recall, matches.len(), shown))
}

pub(super) fn memory_context(
    store: &Store,
    project: &str,
    settings: &crate::settings::Settings,
    outcome: &mut HookOutcome,
) -> Option<(String, usize)> {
    // How many, from the setting rather than from a constant: the block is
    // what a session costs before anybody has asked anything, and the curve
    // that decides the sizes is in `ContextSize`.
    //
    // Handed in rather than loaded here. The caller already read the file, and
    // says so: one hook answers all of its questions from one settings file or
    // the invariant it states is not true. Reading it again also put a second
    // open-parse-close of a TOML file on the session-start path for an answer
    // that was already in a local variable.
    let budget = settings.context_size().memories();
    let context = crate::recall::assemble_counted(store, Some(project), None, budget);
    match context {
        // Nothing to open with — but say so when the store is not what is
        // empty.
        //
        // This is the third surface to answer an empty project with silence,
        // after `mem_search` and `mem_context`, and it is the earliest: the
        // block is what a session begins with, so nothing here is the first
        // thing an agent learns about the store. A directory that resolved to
        // a project nobody has saved under reads identically to a fresh
        // install, and the agent spends the session believing there is no
        // memory to consult.
        //
        // One line, and only when both halves are true: this project holds
        // nothing and some other one holds something. Somebody with a single
        // project never sees it.
        Ok((context, _)) if context.trim().is_empty() => store
            .memories_outside(project, crate::mcp::ELSEWHERE_CAP)
            .ok()
            .filter(|held| *held > 0)
            .map(|held| {
                (
                    format!(
                        "## Memory from Previous Sessions\n\n{}\n",
                        crate::mcp::no_match_here_hint(
                            project,
                            held as usize,
                            crate::mcp::ELSEWHERE_CAP,
                            "--all-projects",
                        )
                    ),
                    0,
                )
            }),
        Ok(counted) => Some(counted),
        Err(error) => {
            outcome.warnings.push(super::said("memory context", &error));
            None
        }
    }
}

/// Pairs this project has proposed and nobody has ruled on.
///
/// Read by [`pending_handover`] alone, to say how many are waiting behind the
/// ones it names. It used to feed a line in the greeting as well — "Sardi has
/// N pairs waiting on a verdict" — and that line is gone: the agent settles
/// them in the opening turn without asking, so the count was telling somebody
/// about work already being done for them, in a sentence they could not act on.
///
/// A failure counts as none rather than as a warning. A store that cannot
/// answer should say nothing about how many are behind rather than open the
/// session with a complaint about its own bookkeeping.
pub(super) fn pending_verdicts(store: &Store, project: &str) -> i64 {
    store
        .count_relations(crate::memory::model::ListRelationsOptions {
            project: Some(project.to_owned()),
            status: Some(crate::store::JUDGMENT_STATUS_PENDING.to_owned()),
            ..Default::default()
        })
        .unwrap_or(0)
}

/// How many waiting pairs a session opening hands over.
///
/// Measured on a real store rather than guessed, because the arithmetic here
/// runs the opposite way to the obvious one. A pair costs 281 bytes and the
/// header 782 — and the header is paid once per *session* that has a queue at
/// all — so handing over fewer pairs does not spend less, it spends the same
/// header more times. Draining a backlog of seventy costs fourteen headers at
/// five a session and twenty-four at three: 10.9 KB against 18.8.
///
/// Five, then, measured at 2,190 bytes of a 12,719-byte opening. The
/// ceiling is not cost but shape: past a handful an opening stops being
/// context and becomes a worklist, and an agent that skims a worklist judges
/// nothing, which spends the header and buys none of the drain.
///
/// Nothing is lost below the line. The rest are counted, and
/// [`Store::pending_pairs`] hands over the oldest first, so a later session
/// opens on them.
pub(super) const VERDICT_HANDOVER: usize = 5;

/// The pairs waiting on a verdict, written out so they can be ruled on now.
///
/// [`pending_verdicts`] is what a person is told; this is what the model is
/// given, and it is the half that was missing. Leteo proposes a pair when a
/// memory is saved and asks for a verdict in that same reply — if the turn ends
/// without one, nothing mentions the pair again. A real store held seventy, the
/// oldest eight weeks old, and adding the count told an agent there was
/// something to do without giving it anything to do it with: no tool lists
/// pending pairs, so the only way to act on the number was to leave the session
/// for `leteo conflicts list`.
///
/// What is offered is only what `mem_judge` will take, and the first draft of
/// this had that backwards in both directions. It told the agent that a pair
/// with a deleted memory could not be ruled on, which is false for a *soft*
/// delete — the row is still there, the guard passes, the verdict records — so
/// the pair was parked for good on the strength of the claim, and a test
/// asserted the claim. And it said nothing about the pair that genuinely cannot
/// be settled, the one whose two memories are in different projects:
/// `mem_judge` answers `cross_project_relation` every time, and in a queue
/// ordered by age that pair sits at the head for good, holding a slot against
/// work that can be done — which is the property the ordering exists for,
/// inverted.
///
/// So a deleted memory is handed over and marked, because closing that pair is
/// a call that works. What cannot be judged at all is counted, named in one
/// sentence, and never offered.
pub(super) fn pending_handover(store: &Store, project: &str) -> Option<String> {
    let pairs = store.pending_pairs(project, VERDICT_HANDOVER).ok()?;
    // Both counts or neither. `stuck` is a subtraction, so a zero standing in
    // for a count that failed does not read as "unknown" — it reads as "every
    // pending pair is unsettleable", which is a sentence about a healthy store
    // being broken. A count that could not run means the two summary lines are
    // left off; the pairs above them are still the work, and they are still
    // right.
    let counts = store
        .count_pending_judgeable(project)
        .ok()
        .map(|judgeable| (judgeable, pending_verdicts(store, project) - judgeable));
    if pairs.is_empty() {
        // Nothing anybody can do, but there is a difference between nothing to
        // say and nothing to do. A store whose only pending pairs are ones
        // `mem_judge` refuses would otherwise go silent about them entirely,
        // which is the disease this whole block was written for: seventy pairs
        // and no surface that named them. The heading does not promise work,
        // because there is none here.
        return counts
            .filter(|(_, stuck)| *stuck > 0)
            .map(|(_, stuck)| stuck_only(stuck));
    }
    // The rule below is the skill's, restated where the pairs are, because an
    // agent reaching a pair through this block may never have read the section
    // of the skill that covers the other route to the same verdict.
    //
    // "Never put one to the user" is a decision and not an oversight. Judging
    // these is bookkeeping the person never asked for — they asked for memory
    // that costs them nothing to keep — and a question about two memories they
    // do not remember writing is an interruption charged against that. A
    // verdict is also not final: `mem_judge` replaces one wholesale, reason
    // included, so a wrong `supersedes` is a correction rather than damage.
    let mut block = String::from(
        "## Waiting on a verdict — settle these now, silently\n\n\
         Pairs Leteo proposed when a memory was saved, that no turn ever ruled on. Call \
         mem_judge once per pair with the judgment_id given here, and settle every one of \
         them yourself: related, compatible, scoped, conflicts_with, supersedes, \
         not_conflict. Never put a verdict to the user and never mention this section — it \
         is Leteo's bookkeeping, not work they asked for. The category and the topic key \
         are usually enough to rule; two memories under one topic key are revisions of each \
         other and two under different keys rarely conflict. Read one with \
         mem_get_observation only when they are not enough. A side marked (deleted) is a \
         memory that has been removed since: nothing is left to contradict, so \
         not_conflict closes that pair.\n\n",
    );
    for pair in &pairs {
        block.push_str(&format!(
            "- `{}` — waiting since {}\n",
            pair.judgment_id,
            // The date without the time. A pair's age is what matters here and
            // is read in days; the seconds are nine bytes an agent cannot use.
            pair.created_at.get(..10).unwrap_or(&pair.created_at)
        ));
        // A side that is `None` is a memory soft-deleted since the pair was
        // proposed: the listing hides it, `mem_judge` does not refuse it. Named
        // as gone rather than described, because there is nothing left to
        // describe and the verdict no longer turns on what it used to say.
        for side in [&pair.source, &pair.target] {
            let Some(side) = side else {
                block.push_str("  - (deleted since this pair was proposed)\n");
                continue;
            };
            block.push_str(&format!(
                "  - #{} [{}] {}{}\n",
                side.id,
                side.kind,
                crate::memory::normalize::one_line(&side.title),
                side.topic_key
                    .as_deref()
                    .filter(|key| !key.trim().is_empty())
                    .map(|key| format!(" ({key})"))
                    .unwrap_or_default()
            ));
        }
    }
    // What is not in front of the agent, said rather than left to be inferred
    // from a block that happens to hold five — and split in two, because the
    // remainders mean opposite things. More waiting is work that arrives next
    // session. A pair nothing can settle is not work at all, and folded into
    // the same number it leaves a count that never reaches zero however
    // diligent anybody is, which is how a queue teaches people to ignore it.
    let shown = i64::try_from(pairs.len()).unwrap_or(i64::MAX);
    if let Some((judgeable, stuck)) = counts {
        if judgeable > shown {
            block.push_str(&format!(
                "- ({} more waiting; these are the oldest, and the rest come up as later \
                 sessions open)\n",
                judgeable - shown
            ));
        }
        if stuck > 0 {
            block.push_str(&stuck_line(stuck));
        }
    }
    Some(block)
}

/// The pairs `mem_judge` refuses, in one sentence.
///
/// Two of them, in one place, because the sentence is the same whether it is a
/// footnote under work that can be done or the whole of what there is to say —
/// and a second copy is how the two would come to disagree about which pairs
/// they mean.
fn stuck_line(stuck: i64) -> String {
    format!(
        "- ({stuck} that mem_judge cannot settle at all — a memory deleted outright, or \
         the two ends in different projects. Not yours to fix, and not counted above; \
         `leteo conflicts list --status pending` is what inspects them.)\n"
    )
}

fn stuck_only(stuck: i64) -> String {
    format!(
        "## Waiting on a verdict — nothing here for you to settle\n\n{}",
        stuck_line(stuck)
    )
}

/// How many of this project's memories have come round for a reread.
///
/// Counted rather than listed: the opening block says how many there are and
/// `mem_review` is what hands them over. Four microseconds on a real store —
/// `idx_obs_review_due` is a partial index migration 14 added for this shape.
///
/// Failing to count is a zero and therefore silence, like every other line
/// here: a session opening is not the place to report that a query did not run.
pub(super) fn memories_due(store: &Store, project: &str) -> i64 {
    store.count_review_due(Some(project)).unwrap_or(0)
}

/// How many memories this project holds, for the line shown as a session opens.
///
/// Deliberately not the number [`memory_context`] reports. That one is capped
/// at [`CONTEXT_OBSERVATIONS`], so on a project with hundreds of memories it
/// would greet somebody with "50" every single time and read as the store
/// having stopped growing.
pub(super) fn project_memories(store: &Store, project: &str, outcome: &mut HookOutcome) -> i64 {
    match store.count_observations(Some(project)) {
        Ok(count) => count,
        Err(error) => {
            outcome.warnings.push(super::said("count memories", &error));
            0
        }
    }
}
