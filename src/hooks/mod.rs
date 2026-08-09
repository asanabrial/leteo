//! Agent lifecycle hooks.
//!
//! Coding agents invoke `leteo hook <event>` at session start, on every user
//! prompt, after context compaction, when a subagent finishes, and when the
//! session stops. Each run reads the agent's JSON payload from standard input,
//! works directly against the local SQLite store, and prints a hook response on
//! standard output.
//!
//! Everything runs inside this binary on purpose. Shell hooks that shell out to
//! `curl` and `jq` need a running HTTP server, a POSIX shell, and both tools on
//! PATH; none of that holds on a stock Windows machine.
//!
//! What stays here is the shape of an event and the order things happen in.
//! What each event needs lives beside the others that need the same thing:
//! [`session`] works out whose conversation this is and which project it
//! belongs to, [`context`] decides what the agent is handed back, and
//! [`nudge`] owns the reminder and the clock that keeps it civil.

use std::io::Read;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{
    AddPrompt, Store, memory::model::PassiveCapture, memory::normalize, project::detect_project,
};

mod context;
mod nudge;
mod session;
#[cfg(test)]
mod tests;

use context::{memories_due, memory_context, pending_handover, project_memories, prompt_recall};
use nudge::{SessionState, nudge_state_path, save_nudge, sweep_stale_nudges};
use session::{
    ensure_session, migrate_directory_project, resolve_directory, resolve_project,
    resolve_session_id,
};

/// Lifecycle events Leteo reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    PostCompaction,
    UserPromptSubmit,
    SubagentStop,
    SessionStop,
}

impl HookEvent {
    /// The agent-facing event name reported back in the hook response.
    /// How many seconds the agent that registered this hook waits before it
    /// kills the process.
    ///
    /// These are the numbers `setup::HOOK_EVENTS` writes into every agent's
    /// configuration, and a test holds the two together. They live here because
    /// this is the type that knows which event is running, and because the
    /// store needs them: see [`store_wait`](Self::store_wait).
    pub fn agent_timeout_seconds(self) -> u64 {
        match self {
            Self::SessionStart | Self::PostCompaction | Self::SubagentStop => 10,
            Self::UserPromptSubmit => 5,
            // Capped by Codex, which clamps this event to three seconds.
            Self::SessionStop => 3,
        }
    }

    /// How long this hook may wait for another writer before it gives up.
    ///
    /// Leteo has to give up before the agent does. The store waited five
    /// seconds for a lock no matter which hook was running, while the agent
    /// kills `session-stop` at three — so a `session-stop` that met a busy
    /// store was killed mid-wait, the session was never ended, and the
    /// reminder's debounce file was left behind, which is the litter the
    /// stale-nudge sweep exists to clear. `user-prompt-submit` was worse in a
    /// quieter way: five against five, so the process was killed at the exact
    /// moment it would have answered.
    ///
    /// A second under the agent's patience — and then a tenth off that, because
    /// a wait of *n* does not take *n*.
    ///
    /// SQLite's busy handler sleeps in a ladder that tops out at 100 ms, and on
    /// Windows a 100 ms sleep is not 100 ms: the timer granularity is 15.6 ms,
    /// so every step overshoots and the error is proportional to how long the
    /// wait is. Measured directly, outside Leteo, against a database held by
    /// another writer:
    ///
    /// ```text
    ///   asked    actual    over
    ///   2000 ms  2263 ms   13.2%
    ///   4000 ms  4450 ms   11.3%
    ///   9000 ms  9927 ms   10.3%
    /// ```
    ///
    /// So the flat second was not a second. `session-start`, told to wait 9
    /// against an agent that waits 10, ran for 9.94 seconds against a store
    /// somebody else held — 58 ms of margin, on this machine, on a good day.
    /// The whole point of the ladder is that Leteo gives up before the agent
    /// does, because a killed hook tells nobody anything, and a margin that
    /// thin is one slow disk away from not existing.
    ///
    /// Nine tenths of what is left after the flat second buys the margin back.
    /// Measured with the store held by another writer for longer than any
    /// budget, worst of two runs each:
    ///
    /// ```text
    ///                        before   after   patience
    ///   session-start        9942 ms  9453 ms   10 s
    ///   subagent-stop        9957 ms  8949 ms   10 s
    ///   user-prompt-submit   4474 ms  4048 ms    5 s
    ///   session-stop         2294 ms  2075 ms    3 s
    /// ```
    ///
    /// `session-start` keeps the thinnest margin of the four at 0.55 s, because
    /// it is the one that builds the whole opening context after the write it
    /// could not do. The other three land within a second of the intent.
    pub fn store_wait(self) -> std::time::Duration {
        let budget =
            std::time::Duration::from_secs(self.agent_timeout_seconds().saturating_sub(1).max(1));
        budget * 9 / 10
    }

    pub fn hook_event_name(self) -> &'static str {
        match self {
            Self::SessionStart | Self::PostCompaction => "SessionStart",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::SubagentStop => "SubagentStop",
            Self::SessionStop => "SessionEnd",
        }
    }
}

/// What a store error reads as in a warning a person will see.
///
/// A busy store is the one failure with a next step: the write did not happen,
/// nothing is half-written, and doing it again in a moment is the whole of the
/// remedy. The tool surface has said so since `store_busy` was added; the hooks
/// printed whatever SQLite said, so somebody who typed two prompts at once read
/// `create session: database error: database is locked` in their terminal —
/// which is the prose of a corrupt file, about a store that was merely in use.
///
/// Measured with the store held by another writer for twenty seconds: every
/// hook still answers, gives up inside its own budget, and warns. What it warns
/// is the part this fixes.
pub(crate) fn said(what: &str, error: &crate::store::StoreError) -> String {
    if error.is_busy() {
        return format!("{what}: {}", crate::store::StoreError::BUSY_ADVICE);
    }
    format!("{what}: {error}")
}

/// The payload agents send on standard input. Unknown fields are ignored so a
/// client can extend its schema without breaking the hook.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct HookInput {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub prompt: String,
    /// Subagent or tool output, as the OpenCode plugin names it.
    ///
    /// That plugin builds this payload itself and puts the text here. Claude
    /// Code and Codex hand the hook their own schema and use
    /// [`last_assistant_message`] instead. Read them through [`output`], never
    /// directly.
    ///
    /// [`last_assistant_message`]: HookInput::last_assistant_message
    /// [`output`]: HookInput::output
    #[serde(default)]
    pub stdout: String,
    /// The same text, as Claude Code and Codex name it.
    ///
    /// Their documentation says in as many words that a hook wanting the final
    /// text of a subagent's turn should read this rather than the transcript,
    /// which is written asynchronously and lags.
    ///
    /// Reading only `stdout` meant passive capture never ran anywhere but
    /// OpenCode. The field defaults, so the payload parsed, the hook reported
    /// success, and `observations_captured` was 0 on every subagent — which is
    /// also what "the subagent said nothing worth keeping" looks like. A real
    /// store of 3,530 memories held not one passively captured memory.
    #[serde(default)]
    pub last_assistant_message: String,
    #[serde(default)]
    pub project: Option<String>,
    /// What produced the text, as Codex names it on its session events.
    #[serde(default)]
    pub source: Option<String>,
    /// The subagent's own name — `Explore`, or whatever it was called.
    ///
    /// Recorded as the memory's `tool_name`, because it says more than the
    /// event that carried the text.
    #[serde(default)]
    pub agent_type: Option<String>,
}

impl HookInput {
    /// The text a subagent finished with, under whichever name it arrived.
    ///
    /// Two fields rather than one field with a `serde` alias, and that
    /// distinction is a bug that shipped. An alias makes both spellings write
    /// the *same* field, so a payload carrying both is a duplicate field and
    /// serde rejects **the whole document**. Codex's hook payload carries
    /// `source` and `agent_type` together, and with them aliased onto one field
    /// every Codex hook parsed as an empty `HookInput`: no session id, no
    /// prompt, no capture — and no error either, because `read_input` falls
    /// back to defaults so a malformed payload can never block somebody's
    /// prompt. It reported success and did nothing.
    ///
    /// Separate fields cannot collide, and the precedence is stated here once.
    pub fn output(&self) -> &str {
        if self.last_assistant_message.trim().is_empty() {
            &self.stdout
        } else {
            &self.last_assistant_message
        }
    }

    /// What to record as the memory's `tool_name`, or nothing.
    pub fn producer(&self) -> Option<&str> {
        self.agent_type
            .as_deref()
            .or(self.source.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

/// What the hook did, plus the text handed back to the agent.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct HookOutcome {
    pub event: &'static str,
    pub project: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    pub session_created: bool,
    pub prompt_saved: bool,
    pub observations_captured: usize,
    /// Why nothing was captured, when nothing was.
    ///
    /// Zero saved has two reasons and the agent-facing silence is right about
    /// both — a workflow finishing dozens of subagents cannot afford a line
    /// each. Somebody running `--verbose` is asking the opposite question, and
    /// "0 captured" answered it with the same word for "the subagent left no
    /// learnings section" and "everything it wrote was already stored". These
    /// come from the same `PassiveCaptureResult` the count comes from, and were
    /// being dropped on the floor between the store and here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observations_extracted: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observations_duplicate: Option<usize>,
    /// Learnings this turn left behind that Leteo did not keep.
    ///
    /// See `normalize::MAX_LEARNINGS` for why there is a bound at all. Reported
    /// beside the other two rather than folded into them: "not kept because
    /// there were too many" is a different fact from "already stored", and an
    /// agent reading `0 captured, 3 duplicate` and one reading
    /// `0 captured, 420 dropped` have different things to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observations_dropped: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl HookOutcome {
    /// Renders the JSON an agent expects on standard output.
    pub fn response(&self) -> serde_json::Value {
        let mut response = serde_json::Map::new();
        if let Some(context) = &self.additional_context {
            response.insert(
                "hookSpecificOutput".to_owned(),
                serde_json::json!({
                    "hookEventName": self.event,
                    "additionalContext": context,
                }),
            );
        }
        if let Some(message) = &self.system_message {
            response.insert(
                "systemMessage".to_owned(),
                serde_json::Value::String(message.clone()),
            );
        }
        serde_json::Value::Object(response)
    }
}

/// Reads a hook payload from standard input, tolerating an empty body.
pub fn read_input(mut reader: impl Read) -> Result<HookInput> {
    let mut body = String::new();
    reader
        .read_to_string(&mut body)
        .context("read hook input")?;
    if body.trim().is_empty() {
        return Ok(HookInput::default());
    }
    let input: HookInput = serde_json::from_str(&body).context("hook input is not valid JSON")?;
    // A payload that said something none of which arrived.
    //
    // Every field here defaults and there is no `deny_unknown_fields`, which is
    // deliberate: a client adds fields to its own schema and a hook that
    // refused them would stop working for a change that was none of its
    // business. The cost is that a payload naming its fields differently — the
    // same JSON in camelCase, or a schema that moved — parses perfectly into an
    // empty `HookInput`, and every hook then reports success having done
    // nothing. That is not hypothetical: a `serde` alias once turned Codex's
    // ordinary payload into a duplicate field, the store filled with sessions
    // nobody could find and prompts that were never saved, and nothing
    // anywhere said why. The warning added afterwards covers a payload that is
    // not JSON, which is the half that announces itself.
    //
    // So the other half is said here, and the test is exact rather than a list
    // of field names to keep in step: the body was an object with something in
    // it, and what came out is indistinguishable from an empty payload.
    if input == HookInput::default()
        && serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&body)
            .is_ok_and(|fields| !fields.is_empty())
    {
        anyhow::bail!(
            "hook payload parsed but carried nothing Leteo reads; check the field names \
             against session_id, cwd, prompt, last_assistant_message"
        );
    }
    Ok(input)
}

/// Runs one lifecycle event against the store.
///
/// Hooks sit on the agent's critical path, so recoverable problems are
/// collected as warnings instead of aborting the run.
pub fn run(store: &mut Store, event: HookEvent, input: &HookInput) -> Result<HookOutcome> {
    let directory = resolve_directory(&input.cwd);
    let detection = detect_project(&directory);
    let mut project = resolve_project(input, &detection);
    let session_id = resolve_session_id(input, &project);

    // An existing session owns its project, exactly as the MCP tools treat it.
    // An agent that changes directory mid-session would otherwise split one
    // conversation's memories across two projects, and neither would be whole.
    if let Ok(session) = store.get_session(&session_id) {
        let session_project = normalize::project(&session.project);
        if !session_project.is_empty() && session_project != project {
            project = session_project;
        }
    }
    let mut outcome = HookOutcome {
        event: event.hook_event_name(),
        project: project.clone(),
        session_id: session_id.clone(),
        ..HookOutcome::default()
    };
    if project.is_empty() {
        outcome
            .warnings
            .push("could not determine the project for this hook".to_owned());
        return Ok(outcome);
    }

    // What detection could not finish, said where it can be read. A hook is
    // where the answer sticks: `ensure_session` writes the project onto the
    // session, and every MCP call carrying that session id inherits it for the
    // rest of the conversation. So a scan that ran out of time and guessed the
    // basename decides where a whole conversation's memories are filed, and
    // until now the only surface that carried the warning at all was
    // `mem_current_project` — a tool nobody calls when nothing looks wrong.
    //
    // Only when the guess is what the hook ended up using. An agent that names
    // its own project, and a session that already owns one, both win over
    // detection above — and warning that a guess may be wrong when nothing is
    // resting on the guess is how a warnings list becomes something people
    // scroll past. Compared against the project actually resolved rather than
    // against the input alone, because the session override happens between the
    // two and is the commoner of the two ways detection gets overruled.
    //
    // And only the warnings about a scan that could not finish — see
    // `ProjectDetection::scan_warning`, which is where that rule lives.
    if project == normalize::project(&detection.project)
        && input
            .project
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        && let Some(warning) = detection.scan_warning()
    {
        outcome.warnings.push(warning.to_owned());
    }

    // Read once per run rather than at each place a line is built, so one hook
    // cannot answer half its questions from one setting and half from another
    // if somebody saves the file mid-event.
    let settings = crate::settings::load_beside(store.database_path());
    let voice = settings.voice;
    // Which language to say it in. Only the lines a person reads are
    // translated: `additional_context` below is protocol handed to a model, and
    // it stays English on purpose — `language_directive` already tells the agent
    // which language to *write* in, which is the part that was ever in doubt.
    // Sardi's own, which is Leteo's unless it has been given one — its lines
    // land in an agent's conversation rather than on Leteo's screens, and that
    // is somewhere the two can reasonably differ.
    let language = settings.voice_language();

    match event {
        HookEvent::SessionStart => {
            migrate_directory_project(store, &directory, &project, &mut outcome);
            ensure_session(store, &session_id, &project, &directory, &mut outcome);
            sweep_stale_nudges(store);
            let mut context = crate::setup::MEMORY_DIRECTIVE.to_owned();
            // Which language to write memories in, said every session.
            //
            // It cannot live in the skill: the skill ships identical to
            // everybody and this answer is per person. Said here it reaches
            // both routes — the plugin bundle and `leteo setup --hooks` — and
            // reflects the settings file as it is now rather than as it was
            // when somebody installed.
            context.push_str(
                "

",
            );
            context.push_str(&settings.language_directive());
            if let Some((memory, _)) = memory_context(store, &project, &settings, &mut outcome) {
                context.push_str("\n\n");
                context.push_str(&memory);
            }
            // The pairs themselves, not only how many there are. The line below
            // says the number to whoever is watching; this hands the agent
            // enough to rule on them without leaving the session, which is what
            // the number never did — `mem_judge` takes a `judgment_id` that
            // only `mem_save` ever returned, so a pair missed at the moment it
            // was proposed had no route back.
            //
            // Outside `voice.reports()` on purpose, and it is the same split
            // `Voice` already draws for the save reminder: silencing Sardi
            // silences what Sardi *says about itself*, not the protocol handed
            // to a model. A quiet Leteo still has to be a correct one.
            if let Some(handover) = pending_handover(store, &project) {
                context.push_str("\n\n");
                context.push_str(&handover);
            }
            outcome.additional_context = Some(context);
            // What the project holds, said once as it opens. A store with
            // nothing in it says nothing: greeting somebody with a zero on
            // their first session is an error report for a store that is only
            // new.
            //
            // The waiting verdicts go here and nowhere else. Leteo proposes a
            // pair on the way in and asks the agent to rule on it; if that turn
            // ends without one, nothing mentions it again. Saying it at every
            // prompt would nag, and saying it never is how seventy of them
            // reached two months old unnoticed — a session opening is once.
            if voice.reports() {
                let lines = [
                    crate::sardi::remembers(
                        language,
                        project_memories(store, &project, &mut outcome),
                    ),
                    // And the memories whose own clock says to read them again.
                    // The dates are set on every decision, policy and
                    // preference and migration 15 rewrote all of them; nothing
                    // ever said the queue existed. `mem_review` reads it, the
                    // skill lists that tool without saying when to reach for
                    // it, and the command line has no equivalent. Here for the
                    // reason the verdicts are: once, as a session opens.
                    crate::sardi::due(language, memories_due(store, &project)),
                ];
                let said = lines.into_iter().flatten().collect::<Vec<_>>().join("\n");
                outcome.system_message = (!said.is_empty()).then_some(said);
            }
        }
        HookEvent::PostCompaction => {
            ensure_session(store, &session_id, &project, &directory, &mut outcome);
            // Everything the hint named before this is gone from the agent's
            // context, so it is worth naming again. Only the list is cleared;
            // the reminder's clock is about how long somebody has been quiet,
            // which a compaction says nothing about.
            let state_path = nudge_state_path(store, &session_id);
            let mut state = SessionState::read(state_path.as_ref());
            if !state.shown.is_empty() {
                state.shown.clear();
                if let Err(error) = state.write(state_path.as_ref()) {
                    outcome.warnings.push(format!("hint state: {error}"));
                }
            }
            let mut context = String::from(
                "Context was compacted. Persist the compacted summary with \
                 mem_session_summary, then continue from the memory below.\n\n",
            );
            // Repeated here because a compaction is precisely when the
            // session-start instruction is gone. An agent that came back
            // without it writes the rest of the conversation's memories in
            // English, and nothing says why.
            context.push_str(&settings.language_directive());
            context.push_str("\n\n");
            match memory_context(store, &project, &settings, &mut outcome) {
                Some((memory, listed)) => {
                    context.push_str(&memory);
                    // The count is what the rebuilt context actually names, so
                    // somebody who just watched a conversation get compacted
                    // can see how much of it came back.
                    if voice.reports() {
                        outcome.system_message = crate::sardi::restored(language, listed);
                    }
                }
                None => context.push_str("No previous memory found for this project."),
            }
            outcome.additional_context = Some(context);
        }
        HookEvent::UserPromptSubmit => {
            ensure_session(store, &session_id, &project, &directory, &mut outcome);
            if !input.prompt.trim().is_empty() {
                match store.add_prompt(AddPrompt {
                    session_id: session_id.clone(),
                    content: input.prompt.clone(),
                    project: Some(project.clone()),
                }) {
                    Ok(_) => outcome.prompt_saved = true,
                    Err(error) => outcome.warnings.push(said("save prompt", &error)),
                }
            }
            // What this conversation has already been handed, so the hint does
            // not hand it again — see `SessionState`.
            //
            // Read once here and written once below, with the reminder handed
            // the same value rather than reading the file for itself. Two
            // read-modify-write cycles over one file inside one process is how
            // an update gets lost: the second reader has to see the first
            // writer's bytes, and nothing but the order of two statements was
            // making that true.
            let state_path = nudge_state_path(store, &session_id);
            let mut state = SessionState::read(state_path.as_ref());
            let recall = prompt_recall(store, &input.prompt, &project, &state.shown);
            outcome.additional_context = recall.as_ref().map(|(context, _, _)| context.clone());
            if let Some((_, _, shown)) = &recall {
                state.remember(shown.iter().copied());
            }
            // One line per prompt at most, and the reminder outranks the hint.
            // The reminder is an instruction with something to do about it; the
            // hint is a maybe, and stacking both is how a prompt ends up with
            // more chrome above it than answer below.
            //
            // With reminders off, `save_nudge` is not called at all rather than
            // called and discarded: it stamps the debounce clock as a side
            // effect, and a silenced Leteo that kept stamping it would greet
            // somebody with a reminder the moment they turned the voice back up.
            let reminder = if voice.reminders() {
                save_nudge(store, language, &project, &session_id, &mut state)
            } else {
                None
            };
            if let Err(error) = state.write(state_path.as_ref()) {
                outcome.warnings.push(format!("hint state: {error}"));
            }
            let hint = if voice.reports() {
                crate::sardi::recalls(language, recall.map_or(0, |(_, count, _)| count))
            } else {
                None
            };
            outcome.system_message = reminder.or(hint);
        }
        HookEvent::SubagentStop => {
            let mut lost_capture = None;
            let output = input.output();
            if !output.trim().is_empty() {
                ensure_session(store, &session_id, &project, &directory, &mut outcome);
                match store.passive_capture(PassiveCapture {
                    session_id: session_id.clone(),
                    content: output.to_owned(),
                    project: project.clone(),
                    source: input.producer().unwrap_or("subagent-stop").to_owned(),
                }) {
                    Ok(result) => {
                        outcome.observations_captured = result.saved;
                        outcome.observations_extracted = Some(result.extracted);
                        outcome.observations_duplicate = Some(result.duplicates);
                        outcome.observations_dropped = Some(result.dropped);
                        // The same reasoning as a capture the store refused: the
                        // subagent's context is gone, and the agent reading this
                        // still has the text, so it is the one thing here it can
                        // do something about. Said whatever the voice setting
                        // is, because it is not a report about memories — it is
                        // a thing to do.
                        if result.dropped > 0 {
                            let rest = if result.dropped == 1 {
                                "one was not stored. If it matters, save it".to_owned()
                            } else {
                                format!(
                                    "{} were not stored. If any of them matter, save them",
                                    result.dropped
                                )
                            };
                            lost_capture = Some(format!(
                                "This subagent left {} learnings and Leteo kept the first {}; \
                                 {rest} with mem_save while you still have the text.",
                                result.extracted,
                                crate::memory::normalize::MAX_LEARNINGS,
                            ));
                        }
                    }
                    Err(error) => {
                        // The one silence here that costs something that cannot
                        // be got back.
                        //
                        // A subagent's learnings live in the text this hook was
                        // handed and nowhere else: it finishes, its context is
                        // discarded, and what it found is gone. Every other
                        // event that loses to a busy store loses a prompt or a
                        // clock, and the parent agent can do nothing about
                        // either. Here it still has the words, so it is the one
                        // case where saying so is worth a line — and the line
                        // says what to do rather than what happened.
                        //
                        // Whatever refused the write: the learnings are gone
                        // either way, and which of the two sentences comes back
                        // is `capture_lost`'s to decide — a busy store is the
                        // one cause worth being sent to retry.
                        lost_capture = Some(error.capture_lost());
                        outcome.warnings.push(said("passive capture", &error));
                    }
                }
            }
            // Silent unless something was actually kept. A workflow can finish
            // dozens of subagents in one turn, and most of them report nothing
            // worth storing; a line each would bury the turn's answer.
            //
            // The exception is above: a capture the store was too busy to take
            // is work that disappears with the subagent's context, and the
            // agent reading this still has the text to send again. That one is
            // said whatever the voice setting is, because it is not a report
            // about memories — it is a thing to do.
            if let Some(lost) = lost_capture {
                outcome.system_message = Some(lost);
            } else if voice.reports() {
                outcome.system_message =
                    crate::sardi::captured(language, outcome.observations_captured);
            }
        }
        HookEvent::SessionStop => {
            if store.get_session(&session_id).is_ok()
                && let Err(error) = store.end_session(&session_id, None)
            {
                outcome.warnings.push(said("end session", &error));
            }
            // The reminder clock dies with the session. Without this, every
            // conversation ever held would leave a file behind forever.
            if let Some(state_path) = nudge_state_path(store, &session_id) {
                let _ = std::fs::remove_file(state_path);
            }
        }
    }
    Ok(outcome)
}
