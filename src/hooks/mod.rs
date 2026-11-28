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
pub(crate) mod spool;
#[cfg(test)]
mod tests;

use context::{
    Recalled, UNREADABLE_BLOCK, memories_due, memory_context, pending_handover, project_memories,
    prompt_recall,
};
use nudge::{SessionState, nudge_state_path, save_nudge, sweep_stale_nudges};
use session::{
    drifted_directory_projects, ensure_session, migrate_directory_project, resolve_directory,
    resolve_project, resolve_session_id,
};

/// What Pi sets in the environment of every process it starts.
pub const PI_HOST_VARIABLE: &str = "PI_CODING_AGENT";

/// What the Pi package sets on the hooks it runs itself, and the value it uses.
pub const HOOK_CLIENT_VARIABLE: &str = "LETEO_HOOK_CLIENT";
pub const PI_HOOK_CLIENT: &str = "pi";

/// Whether a hook should do nothing because Pi already owns this session.
///
/// Pi runs Claude models through pi-claude-bridge, which loads the user's Claude
/// Code settings, so Claude Code's hooks fire inside a Pi session that the Pi
/// package has already registered and every conversation would be in the store
/// twice. The hooks Claude Code runs are plain `leteo hook <event>` commands
/// that carry nothing of their own to tell them from the Pi package's, and both
/// inherit `PI_CODING_AGENT`; so the package names itself, and it is the only
/// caller let through.
pub fn defers_to_pi(host: Option<&str>, client: Option<&str>) -> bool {
    host.is_some_and(|value| !value.trim().is_empty())
        && client.map(str::trim) != Some(PI_HOOK_CLIENT)
}

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

    /// Shapes the store a hook runs against.
    ///
    /// Three things differ from every other caller, and each is here rather
    /// than in the command-line wiring because a hook is what they are about.
    ///
    /// The wait is the event's — see [`store_wait`](Self::store_wait) — and it
    /// is one budget for the whole process, because an event that writes twice
    /// must not spend it twice.
    ///
    /// And no vector is made on the write path: making one loads a 13 MB model
    /// and runs it, inside the budget the agent is counting down, where the
    /// backfill can do the same work afterwards with nothing waiting. Measured
    /// on a copy of a real store with the model present, a `subagent-stop` with
    /// five learnings took 108 ms and takes 70 ms, medians of eight runs. The
    /// five memories are written either way; the 38 ms bought a vector a search
    /// would have waited for instead of an answer an agent was waiting for.
    pub fn store_config(config: &mut crate::store::StoreConfig, event: Self) {
        config.busy_timeout = event.store_wait();
        config.single_busy_budget = true;
        config.embed_on_write = false;
    }

    /// Whether this event replays what a busy store left in the spool.
    ///
    /// The replay used to follow every event, and it is bounded only by the
    /// hook's own budget, so a backlog was paid for by whoever was waiting on
    /// that hook. Measured on a debug build against a temporary store, a replay
    /// costs about 2 ms per queued capture (100 in 0.2 s, 1,000 in 2.2 s, the
    /// spool's cap), which on a busy day is most of the 3.6 seconds the prompt
    /// hook may spend. So the two events a person waits behind, the prompt and
    /// the compaction, leave the spool alone.
    ///
    /// Nothing is stranded by that. Only `subagent-stop` writes the spool, and
    /// it replays on its own way out; `session-stop` and the next
    /// `session-start` follow, and the MCP server replays when it starts.
    /// ZCode, whose client fires neither `SubagentStop` nor `SessionEnd`, has
    /// no spooling event of its own to strand.
    pub fn replays_spool(self) -> bool {
        match self {
            Self::SessionStart | Self::SubagentStop | Self::SessionStop => true,
            Self::PostCompaction | Self::UserPromptSubmit => false,
        }
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
    /// Leave the memory directive out of a `session-start` answer.
    ///
    /// For a host that injects the protocol itself on every turn: OpenCode and
    /// Pi each carry a protocol of their own, and the directive on top was about
    /// 925 bytes said twice in Pi. They used to ask `post-compaction` for the
    /// bare memory block instead, which opened every fresh OpenCode session
    /// with "Context was compacted" and an invitation to summarise nothing.
    /// Everything else a session opening carries stays: the language, the
    /// memory, and the verdicts waiting for a ruling.
    #[serde(default)]
    pub without_directive: bool,
    /// The host has already stored the compaction summary itself.
    ///
    /// Pi saves the summary before it runs `post-compaction`, and the recovery
    /// text then told the agent to persist it with `mem_session_summary` — the
    /// same summary written twice. Only a host that did the write can say so;
    /// every other caller leaves it off and gets the instruction it always had.
    #[serde(default)]
    pub summary_stored: bool,
    /// What this call hands the agent does not outlast the turn it is handed
    /// for, so the hook must not record it as delivered.
    ///
    /// Pi puts the hint and the opening block into the system prompt of one run,
    /// and the next prompt starts from a prompt without them; OpenCode does the
    /// same with the hint. Recording the ids in the session state anyway
    /// suppressed a memory for the rest of the session on a host that no longer
    /// held it. A host sets this on the calls whose answer it forgets and leaves
    /// it off on the ones it keeps: OpenCode keeps its opening block, so it sets
    /// the flag on prompts only.
    #[serde(default)]
    pub transient_hints: bool,
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

/// One phase of a hook run, as the milliseconds it took.
///
/// Recorded on every run and printed by `leteo hook <event> --verbose` on
/// standard error, never on the standard output the agent reads. A hook that is
/// slow or killed at its budget leaves nothing else that says which part was
/// slow, and the store it is slow on is the user's own — so the numbers have to
/// come from the run that is happening.
///
/// `write` covers the whole event handler, and `recall` is the part of it spent
/// assembling the opening block: they overlap on purpose, because "the store is
/// slow to read from" and "the event is slow" are different diagnoses and
/// subtracting one from the other is arithmetic nobody should have to do while
/// reading a log line.
///
/// A killed hook still leaves nothing — a process the agent ended cannot print
/// — so this is for the slow run that survived, which is the one somebody can
/// be asked to re-run with `--verbose`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseTime {
    /// The phase's name, as `--verbose` prints it.
    pub phase: &'static str,
    /// How long it took, rounded down to the millisecond.
    pub millis: u64,
}

impl PhaseTime {
    pub(crate) fn since(phase: &'static str, started: std::time::Instant) -> Self {
        Self {
            phase,
            millis: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        }
    }
}

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
    /// How long each phase took. See [`PhaseTime`].
    ///
    /// Not serialised at all: `--verbose` prints these on standard error, and
    /// standard output is the agent's channel. A field here would put them in
    /// the outcome `--verbose` prints, which is still standard output.
    #[serde(skip)]
    pub timings: Vec<PhaseTime>,
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

/// What this directory's sessions were filed under, newest first.
///
/// A detection is a folder's name; the sessions that already live there are
/// evidence about which project the directory really is. The live path and a
/// spool replay both ask this, so a capture the busy-open path had to guess can
/// land where the conversation already lives.
fn recorded_project(store: &Store, project: String, directory: &str) -> String {
    match store.recent_projects_in_directory(directory, &project) {
        Ok(recorded) => recorded.into_iter().next().unwrap_or(project),
        Err(_) => project,
    }
}

/// The project an existing session owns, or `project` when there is none.
///
/// An agent that changes directory mid-session would otherwise split one
/// conversation's memories across two projects, and neither would be whole.
fn session_project(store: &Store, session_id: &str, project: String) -> String {
    match store.get_session(session_id) {
        Ok(session) => {
            let owned = normalize::project(&session.project);
            if owned.is_empty() || owned == project {
                project
            } else {
                owned
            }
        }
        Err(_) => project,
    }
}

/// The capture a `subagent-stop` payload describes, worked out without a store.
///
/// The CLI reads the payload before it opens the store, so a capture whose open
/// fails busy can still be spooled — and that path has no session to override
/// the project from, because there is no store to ask. What it could not settle
/// travels with the entry, so a replay finishes the same resolution with the
/// store open rather than keeping the folder's name.
pub(crate) fn pending_capture(
    input: &HookInput,
) -> Option<(PassiveCapture, String, spool::Unsettled)> {
    if input.output().trim().is_empty() {
        return None;
    }
    let directory = resolve_directory(&input.cwd);
    let detection = detect_project(&directory);
    // The reasons are dropped here: a capture that reached the spool is one
    // whose store could not be opened, and the replay through `run_event` asks
    // the same questions again with somewhere to put the answer.
    let (project, _) = resolve_project(input, &detection);
    let (session_id, _) = resolve_session_id(input, &project);
    let named_by_agent = !input
        .project
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty();
    let refused_session = normalize::session_id_refusal(input.session_id.trim()).is_some();
    let unsettled = spool::Unsettled {
        guessed: !named_by_agent && detection.is_guess(),
        // A refused id falls back to the derived one here, and the replay has
        // to fall back too: carrying the refused id through would make
        // `create_session` refuse the replay and drop the entry, which is the
        // one copy of a subagent's learning.
        derived_session: input.session_id.trim().is_empty() || refused_session,
    };
    Some((
        capture_of(input, &session_id, &project),
        directory.to_string_lossy().into_owned(),
        unsettled,
    ))
}

/// The capture a payload stores, under the session and project already resolved.
///
/// One place for the default producer name, because both doors into the spool —
/// a hook that reached the store and one that could not open it — have to record
/// the same text for the same subagent.
fn capture_of(input: &HookInput, session_id: &str, project: &str) -> PassiveCapture {
    PassiveCapture {
        session_id: session_id.to_owned(),
        content: input.output().to_owned(),
        project: project.to_owned(),
        source: input.producer().unwrap_or("subagent-stop").to_owned(),
    }
}

/// Runs one lifecycle event against the store.
///
/// Hooks sit on the agent's critical path, so recoverable problems are
/// collected as warnings instead of aborting the run.
pub fn run(store: &mut Store, event: HookEvent, input: &HookInput) -> Result<HookOutcome> {
    let write = std::time::Instant::now();
    let mut outcome = run_event(store, event, input)?;
    outcome.timings.push(PhaseTime::since("write", write));
    if event.replays_spool() {
        // Bounded by what is left of this hook's own budget, so the drain
        // cannot outlast the agent that is waiting for the answer.
        let deadline = store.wait_deadline();
        let drain = std::time::Instant::now();
        spool::drain(store, deadline);
        outcome.timings.push(PhaseTime::since("spool", drain));
    }
    Ok(outcome)
}

fn run_event(store: &mut Store, event: HookEvent, input: &HookInput) -> Result<HookOutcome> {
    let detected = std::time::Instant::now();
    let directory = resolve_directory(&input.cwd);
    let detection = detect_project(&directory);
    let (mut project, project_reason) = resolve_project(input, &detection);
    let named_by_agent = !input
        .project
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty();
    let guessed = !named_by_agent && detection.is_guess();
    // A guess is a folder's name, and what this directory's sessions were filed
    // under is evidence. It has to be settled before the session exists, because
    // the session then owns its project: a removed worktree opening one under its
    // folder name files the whole conversation there.
    if guessed {
        project = recorded_project(store, project, &directory.to_string_lossy());
    }
    let (mut session_id, session_reason) = resolve_session_id(input, &project);
    // Whether the id came from the payload or was derived for this hook. A
    // refused id is derived too, and the drift fold below can change the
    // project it was derived from: re-deriving only the blank case filed one
    // capture under two sessions, one from the guess and one from the recorded
    // project the replay used.
    let derived_session = input.session_id.trim().is_empty() || session_reason.is_some();

    // An existing session owns its project, exactly as the MCP tools treat it.
    // An agent that changes directory mid-session would otherwise split one
    // conversation's memories across two projects, and neither would be whole.
    project = session_project(store, &session_id, project);
    let mut outcome = HookOutcome {
        event: event.hook_event_name(),
        project: project.clone(),
        session_id: session_id.clone(),
        ..HookOutcome::default()
    };
    outcome.warnings.extend(project_reason);
    outcome.warnings.extend(session_reason);
    if project.is_empty() {
        outcome
            .warnings
            .push("could not determine the project for this hook".to_owned());
        return Ok(outcome);
    }

    // A directory whose history is under another name is a question for the
    // user, and a hook cannot ask it. What it can do is refuse to make the
    // dispute permanent: opening a session under the newly detected name while
    // the memories stay under the old one means the next write is refused as
    // `ambiguous_project`, in both directions, and the person is never told
    // why. So the session joins the recorded name — the history is the
    // evidence — and the question goes into the context the agent reads.
    //
    // The one drift a hook repairs without asking is a repository that took
    // its directory's name: its memories were filed under the plain basename,
    // and that name is not a project anyone chose, so they move to what the
    // remote now says. Any other recorded name is a project somebody worked
    // under — an old remote, a fork — and moving it would answer the question
    // the warning exists to ask. Neither decision may look at a project the
    // payload names: that name belongs to this session alone, and using it as
    // the fold's target once took a directory's whole history with it.
    let detected_project = normalize::project(&detection.project);
    let mut drifted: Vec<String> = Vec::new();
    if matches!(event, HookEvent::SessionStart) && !named_by_agent {
        let basename = directory
            .file_name()
            .map(|name| normalize::project(&name.to_string_lossy()))
            .unwrap_or_default();
        match drifted_directory_projects(store, &directory, &project) {
            Ok(found) => drifted = found,
            // A lookup that could not run is not a lookup that found no drift,
            // so it says which it was rather than passing as agreement.
            Err(error) => outcome
                .warnings
                .push(said("check this directory's project history", &error)),
        }
        let plain_name = drifted.len() == 1 && drifted.first() == Some(&basename);
        if plain_name {
            migrate_directory_project(store, &directory, &detected_project, &mut outcome);
            // The fold can leave sessions behind when the same name was also
            // recorded from another checkout. Those are still drift, and the
            // warning is the only thing that says so.
            match drifted_directory_projects(store, &directory, &project) {
                Ok(found) => drifted = found,
                Err(error) => outcome
                    .warnings
                    .push(said("check this directory's project history", &error)),
            }
        }
        if let Some(recorded) = drifted.first() {
            project = recorded.clone();
            outcome.project = project.clone();
            if derived_session {
                let (derived, reason) = resolve_session_id(input, &project);
                session_id = derived;
                outcome.session_id = session_id.clone();
                outcome.warnings.extend(reason);
            }
        }
    }
    if !drifted.is_empty() {
        outcome.warnings.push(format!(
            "this directory's sessions were recorded under {}, and it now resolves to \
             {detected_project:?}; ask the user which project this belongs to, or fold the two \
             together with `leteo projects consolidate`",
            drifted.join(", ")
        ));
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
    // And only the warnings about detection that could not finish — see
    // `ProjectDetection::unfinished_warning`, which is where that rule lives.
    if project == detected_project
        && !named_by_agent
        && let Some(warning) = detection.unfinished_warning()
    {
        outcome.warnings.push(warning.to_owned());
    }

    // Read once per run rather than at each place a line is built, so one hook
    // cannot answer half its questions from one setting and half from another
    // if somebody saves the file mid-event.
    let settings = crate::settings::load_beside(store.database_path());
    // Everything above is detection: the directory, what it resolves to, and
    // which session owns it.
    outcome.timings.push(PhaseTime::since("detect", detected));
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
            ensure_session(store, &session_id, &project, &directory, &mut outcome);
            sweep_stale_nudges(store);
            let mut context = if input.without_directive {
                String::new()
            } else {
                let mut directive = crate::setup::MEMORY_DIRECTIVE.to_owned();
                directive.push_str("\n\n");
                directive
            };
            // Which language to write memories in, said every session.
            //
            // It cannot live in the skill: the skill ships identical to
            // everybody and this answer is per person. Said here it reaches
            // both routes — the plugin bundle and `leteo setup --hooks` — and
            // reflects the settings file as it is now rather than as it was
            // when somebody installed.
            context.push_str(&settings.language_directive());
            // The question the hook cannot ask, where the agent reads it. A
            // warning is a surface nobody opens; this one is in the
            // conversation at the moment the agent starts filing memories,
            // which is the only moment it can act on it.
            if !drifted.is_empty() {
                context.push_str(&format!(
                    "\n\nThis directory's sessions were recorded under {}, and the repository \
                     now resolves to {detected_project:?}. The session was opened under the recorded \
                     name so the history stays whole. Ask the user which project this belongs \
                     to, or fold the two together with `leteo projects consolidate`.\n",
                    drifted.join(", ")
                ));
            }
            let recall = std::time::Instant::now();
            let recalled = memory_context(store, &project, &settings, &mut outcome, None);
            outcome.timings.push(PhaseTime::since("recall", recall));
            match recalled {
                Recalled::Found(memory, listed) => {
                    context.push_str("\n\n");
                    context.push_str(&memory);
                    // What the block lists is in the agent's hands from the
                    // first turn, so the hint must not name it again. Added to
                    // whatever the state holds rather than replacing it: a
                    // session start can be a resume, and the hints named before
                    // it are still in the transcript.
                    if !listed.memories.is_empty() && !input.transient_hints {
                        let state_path = nudge_state_path(store, &session_id);
                        let mut state = SessionState::read(state_path.as_ref());
                        state.remember_listed(&listed);
                        if let Err(error) = state.write(state_path.as_ref()) {
                            outcome.warnings.push(format!("hint state: {error}"));
                        }
                    }
                }
                // The opening used to say nothing at all here, which is the
                // same shape as a fresh install. The warning names the cause on
                // stderr; this is the sentence the agent reads.
                Recalled::Unreadable => {
                    context.push_str("\n\n");
                    context.push_str(UNREADABLE_BLOCK);
                }
                Recalled::Empty => {}
            }
            // The pairs themselves, not only how many there are: `mem_judge`
            // takes a `judgment_id` that only `mem_save` ever returned, so a
            // pair missed at the moment it was proposed had no route back
            // without leaving the session.
            //
            // Outside `voice.reports()` on purpose, and it is the same split
            // `Voice` already draws for the save reminder: silencing Sardi
            // silences what Sardi *says about itself*, not the protocol handed
            // to a model. A quiet Leteo still has to be a correct one. The
            // pairs nothing can settle are the other way round — no agent can
            // act on them — and go out in the person's message below.
            let handover = pending_handover(store, &project);
            if let Some(block) = &handover.agent {
                context.push_str("\n\n");
                context.push_str(block);
            }
            outcome.additional_context = Some(context);
            // What the project holds, said once as it opens. A store with
            // nothing in it says nothing: greeting somebody with a zero on
            // their first session is an error report for a store that is only
            // new.
            //
            // The pairs nothing can settle reach the person here and nowhere
            // else: the agent's block says nothing about them because no agent
            // call acts on one, and saying it at every prompt would nag. A
            // session opening is once.
            let mut said: Vec<String> = Vec::new();
            if voice.reports() {
                said.extend(
                    [
                        crate::sardi::remembers(
                            language,
                            project_memories(store, &project, &mut outcome),
                        ),
                        // And the memories whose own clock says to read them
                        // again. The dates are set on every decision, policy
                        // and preference and migration 15 rewrote all of them;
                        // nothing ever said the queue existed. `mem_review`
                        // reads it, the skill lists that tool without saying
                        // when to reach for it, and the command line has no
                        // equivalent. Here for the reason the verdicts are:
                        // once, as a session opens.
                        crate::sardi::due(language, memories_due(store, &project)),
                    ]
                    .into_iter()
                    .flatten(),
                );
            }
            // Not gated on the voice: this is not Sardi talking about itself,
            // it is a queue only the person can clear, and it was sent to an
            // agent that could do nothing with it.
            if let Some(line) = handover.person {
                said.push(line);
            }
            outcome.system_message = (!said.is_empty()).then(|| said.join("\n"));
        }
        HookEvent::PostCompaction => {
            ensure_session(store, &session_id, &project, &directory, &mut outcome);
            // Everything the hint named before this is gone from the agent's
            // context, so it is worth naming again — and so is the sentence
            // that explained it. The state is cleared and then refilled with
            // what the rebuilt block lists, below: the block is the first thing
            // the agent reads again, and a hint naming one of its lines is a
            // repeat of something a few lines up.
            let state_path = nudge_state_path(store, &session_id);
            let mut state = SessionState::read(state_path.as_ref());
            let was_blank = state.is_blank();
            state.forget_context();
            let mut context = String::from(if input.summary_stored {
                "Context was compacted, and its summary is already stored. \
                 Continue from the memory below.\n\n"
            } else {
                "Context was compacted. Persist the compacted summary with \
                 mem_session_summary, then continue from the memory below.\n\n"
            });
            // Repeated here because a compaction is precisely when the
            // session-start instruction is gone. An agent that came back
            // without it writes the rest of the conversation's memories in
            // English, and nothing says why.
            context.push_str(&settings.language_directive());
            context.push_str("\n\n");
            let recall = std::time::Instant::now();
            // This session's own prompts are in the host's compaction summary
            // and in the conversation that was just replaced; listing them back
            // is text the agent has already read.
            let recalled =
                memory_context(store, &project, &settings, &mut outcome, Some(&session_id));
            outcome.timings.push(PhaseTime::since("recall", recall));
            match recalled {
                Recalled::Found(memory, listed) => {
                    context.push_str(&memory);
                    if !input.transient_hints {
                        state.remember_listed(&listed);
                    }
                    // The count is what the rebuilt context actually names, so
                    // somebody who just watched a conversation get compacted
                    // can see how much of it came back.
                    if voice.reports() {
                        outcome.system_message =
                            crate::sardi::restored(language, listed.memories.len());
                    }
                }
                Recalled::Empty => {
                    context.push_str("No previous memory found for this project.");
                }
                Recalled::Unreadable => context.push_str(UNREADABLE_BLOCK),
            }
            // A conversation with no state file and nothing to record does not
            // get one made for it.
            if !(was_blank && state.is_blank())
                && let Err(error) = state.write(state_path.as_ref())
            {
                outcome.warnings.push(format!("hint state: {error}"));
            }
            outcome.additional_context = Some(context);
        }
        HookEvent::UserPromptSubmit => {
            ensure_session(store, &session_id, &project, &directory, &mut outcome);
            // A message the harness wrote is not something somebody asked, so
            // it is neither stored nor searched for a hint.
            let harness = normalize::is_harness_message(&input.prompt);
            if !harness && !input.prompt.trim().is_empty() {
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
            let recall = if harness {
                None
            } else {
                prompt_recall(store, &input.prompt, &project, &state)
            };
            outcome.additional_context = recall.as_ref().map(|recall| recall.context.clone());
            // Not on a host that drops the hint after the turn: what it was
            // told is gone by the next prompt, and the header sentence with it.
            if let Some(recall) = &recall
                && !input.transient_hints
            {
                state.remember(recall.shown.iter().copied());
                state.remember_caveats(recall.caveats.iter().cloned());
                state.headed = true;
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
                crate::sardi::recalls(language, recall.map_or(0, |recall| recall.count))
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
                let capture = capture_of(input, &session_id, &project);
                match store.passive_capture_from(capture.clone(), directory.to_str()) {
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
                    Err(failure) => {
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
                        // A busy store is no longer a loss at all: the capture
                        // is written to the spool beside the database and a
                        // later open replays it, so `capture_lost` is told
                        // whether that worked and says "kept for later" when it
                        // did. A capture that stopped part way has already
                        // committed some learnings, so its count is handed over
                        // too rather than claiming the whole text is gone.
                        let error = failure.error;
                        let spooled = error.is_busy()
                            && spool::spool(
                                store.data_dir(),
                                event.hook_event_name(),
                                &capture,
                                &directory.to_string_lossy(),
                            )
                            .is_ok();
                        lost_capture = Some(error.capture_lost(failure.partial.saved, spooled));
                        outcome.warnings.push(said("passive capture", &error));
                    }
                }
            }
            // Silent unless something was actually kept. A workflow can finish
            // dozens of subagents in one turn, and most of them report nothing
            // worth storing; a line each would bury the turn's answer.
            //
            // The exception is above: a capture the store refused is either
            // kept beside the database for a later open to replay or, for a
            // cause a retry cannot mend, work that disappears with the
            // subagent's context. Either way it is said whatever the voice
            // setting is — one is a thing to do while the text is still here,
            // the other is a thing to know.
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
