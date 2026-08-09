//! The reminder to save, and the clock that keeps it from nagging.
//!
//! One line at most, and only when a project has gone quiet for long enough
//! that something worth keeping has probably happened. The debounce lives in a
//! file per session beside the database, because the hook is a fresh process
//! every time and has nowhere else to remember it.

use std::path::PathBuf;

use crate::Store;

/// Minutes of silence after which the save reminder may fire again.
pub(super) const NUDGE_COOLDOWN_MINUTES: i64 = 15;
/// Minutes a session must run before any reminder is considered.
const NUDGE_WARMUP_MINUTES: i64 = 5;
/// Days after which reminder state is treated as a dead session's leftovers.
const NUDGE_STATE_MAX_AGE_DAYS: u64 = 7;

/// Reminds the agent to persist work when a project has been quiet for a while.
/// The reminder is debounced per session so a session with nothing worth saving
/// is not nagged on every prompt.
pub(super) fn save_nudge(
    store: &Store,
    language: crate::settings::Interface,
    project: &str,
    session_id: &str,
    state: &mut SessionState,
) -> Option<String> {
    let session = store.get_session(session_id).ok()?;
    let started_at = parse_sqlite_timestamp(&session.started_at)?;
    let now = chrono::Utc::now().naive_utc();
    if (now - started_at).num_minutes() < NUDGE_WARMUP_MINUTES {
        return None;
    }
    let last_save = store
        .recent_observations(Some(project), Some(1), true)
        .ok()?
        .first()
        .and_then(|observation| parse_sqlite_timestamp(&observation.created_at));
    // Counted from whichever came later: the last thing saved, or the moment
    // this conversation began.
    //
    // The reminder means "you have been working and have kept none of it", and
    // it was measuring "this project has not been saved to in a while" — which
    // is also true, and says nothing, when the work has not started. Opening a
    // project untouched for a week and typing one sentence fired it, because
    // the project's last memory was a week old; on a real store it announced
    // 7,504 minutes, which is five days in which nothing had happened yet.
    //
    // Measured from the session, the sentence is true again: the number is how
    // long *this* conversation has gone without keeping anything, and the
    // reminder cannot fire before there has been time to learn something.
    let since = match last_save {
        Some(last_save) => last_save.max(started_at),
        None => started_at,
    };
    let quiet_minutes = (now - since).num_minutes();
    if quiet_minutes < NUDGE_COOLDOWN_MINUTES {
        return None;
    }

    // The clock is stamped on the state the caller holds and the caller writes
    // it, because the memories this session has already been handed live in the
    // same file. Reading it again here and writing back a bare timestamp is
    // exactly how that list would be lost.
    if let Some(last_nudge) = state
        .last_nudge
        .as_deref()
        .and_then(|value| parse_sqlite_timestamp(value.trim()))
        && (now - last_nudge).num_minutes() < NUDGE_COOLDOWN_MINUTES
    {
        return None;
    }
    state.last_nudge = Some(crate::timestamp::format(now));
    Some(crate::sardi::nudge(language, project, quiet_minutes))
}

/// Clears reminder state left behind by sessions that never stopped cleanly.
///
/// `SessionEnd` removes the file for a conversation that ends the ordinary way,
/// but a killed terminal or a crashed agent never fires it, and one file per
/// conversation accumulates for the life of the install.
///
/// Only files far too old to belong to a live session are removed. The debounce
/// is written when a reminder fires and never touched again, so a young file may
/// well belong to a session still running, and deleting it would hand that
/// session a reminder on its very next prompt.
pub(super) fn sweep_stale_nudges(store: &Store) {
    let Some(directory) = store
        .database_path()
        .parent()
        .map(|root| root.join("hooks"))
    else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return;
    };
    let cutoff = std::time::Duration::from_secs(NUDGE_STATE_MAX_AGE_DAYS * 24 * 60 * 60);
    for entry in entries.flatten() {
        if entry
            .path()
            .extension()
            .is_none_or(|suffix| suffix != "nudge")
        {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > cutoff);
        if stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Per-session reminder state, kept beside the database so several databases
/// never share one debounce clock.
pub(super) fn nudge_state_path(store: &Store, session_id: &str) -> Option<PathBuf> {
    let safe_session: String = session_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    Some(
        store
            .database_path()
            .parent()?
            .join("hooks")
            .join(format!("{safe_session}.nudge")),
    )
}

pub(super) fn parse_sqlite_timestamp(value: &str) -> Option<chrono::NaiveDateTime> {
    crate::timestamp::parse(value)
}

/// Which memories the per-prompt hint has already handed this conversation.
///
/// The hint is chosen by what the current prompt is about, and a conversation
/// stays about the same thing for a while — so it kept naming memories the
/// agent had already been given. Replayed over six real sessions in five
/// projects, 134 of 273 memories handed over were repeats of one already
/// handed over in the same session: 36%, 36%, 41%, 41%, 58% and 81%.
///
/// A repeat is not free and it is not neutral. It spends the same room as a new
/// one and says "here is something you should know" about something the agent
/// was told twenty minutes ago, which is how a hint that is usually right stops
/// being read.
///
/// It lives in the file the reminder clock already uses, because the hook is a
/// fresh process every time and has nowhere else to remember: one file per
/// session, swept when it outlives any live conversation, deleted when the
/// session ends. A file that cannot be read is not an error — the hint simply
/// repeats itself, which is what it did before.
///
/// Cleared on compaction. The agent's context is rewritten there, so a memory
/// named before it is genuinely gone and naming it again is the right thing.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct SessionState {
    /// When the reminder last fired. `None` on a file written before this
    /// carried anything else, which is read as "no reminder yet" — one extra
    /// reminder on a conversation that was already running, and nothing worse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) last_nudge: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) shown: Vec<i64>,
}

/// The longest conversation anybody has actually held, when this was last
/// measured.
///
/// A fact rather than a decision, and the only thing here that is. Measured on
/// a real store of 1,198 prompts across 135 sessions: half hold one prompt, 90%
/// hold thirteen or fewer, 95% hold twenty-five, and the two longest hold 315
/// and this.
///
/// It is written down so the sizing below has something to be held to. Sized
/// from a number nobody records, the sizing can be lowered back to what it was
/// and every test still passes — which is exactly what happened when this was
/// one constant instead of two.
pub(super) const LONGEST_CONVERSATION_MEASURED: usize = 351;

/// The longest conversation this has to cover.
///
/// The measurement with room, because the next long conversation will be longer
/// than the last one. A number to re-measure rather than a law: raising the
/// figure above is what changes it, and a guard holds this one to it.
pub(super) const LONGEST_CONVERSATION_PROMPTS: usize = 400;

/// The sizing has to cover the measurement, and the compiler is what says so.
///
/// This was a line in a test, which is the wrong place twice over: a test can
/// only fail once somebody runs it, and the constant it reads was otherwise
/// unused, so the build warned about a fact written down on purpose. Held here
/// it cannot be lowered past the measurement at all.
const _: () = assert!(LONGEST_CONVERSATION_PROMPTS >= LONGEST_CONVERSATION_MEASURED);

/// How many memories one conversation remembers having been handed.
///
/// The list is written on every prompt, so it cannot grow without bound — but
/// the bound has to cover a whole conversation, or the promise it exists for
/// stops holding. Past it the oldest ids fall off and become eligible again, so
/// the hint offers the same memory a second time in the same conversation,
/// which is the one thing `hooks.md` §9 says it never does.
///
/// It was 128, sized against sessions of 45 prompts, which was the longest this
/// store held when it was written. It now holds one of 351 — three memories a
/// prompt against that is a thousand, and 128 covers a third of it.
///
/// Derived rather than chosen, so the arithmetic in the paragraph above is the
/// arithmetic in the code. Twelve hundred ids is a 7.2 KB file and 0.07 ms to
/// read and write, against the 13 ms this hook costs end to end; at 128 it was
/// 0.8 KB and 0.01 ms, and the difference is not a thing anybody can measure
/// from outside.
const MAX_REMEMBERED: usize = super::context::RECALL_LIMIT * LONGEST_CONVERSATION_PROMPTS;

impl SessionState {
    pub(super) fn read(path: Option<&PathBuf>) -> Self {
        let Some(contents) = path.and_then(|path| std::fs::read_to_string(path).ok()) else {
            return Self::default();
        };
        // The file used to be a bare timestamp and some are still on disk.
        serde_json::from_str(&contents).unwrap_or_else(|_| Self {
            last_nudge: Some(contents.trim().to_owned()).filter(|value| !value.is_empty()),
            shown: Vec::new(),
        })
    }

    pub(super) fn write(&self, path: Option<&PathBuf>) -> Result<(), std::io::Error> {
        let Some(path) = path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string(self).unwrap_or_default())
    }

    pub(super) fn remember(&mut self, ids: impl IntoIterator<Item = i64>) {
        for id in ids {
            if !self.shown.contains(&id) {
                self.shown.push(id);
            }
        }
        if self.shown.len() > MAX_REMEMBERED {
            self.shown.drain(..self.shown.len() - MAX_REMEMBERED);
        }
    }
}
