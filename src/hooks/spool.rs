//! Captures kept beside the store when it is too busy to take them.
//!
//! A hook runs on the agent's critical path and gives up on a locked store
//! before the agent kills it. Every event it loses is bookkeeping except one: a
//! `subagent-stop` capture is the text a subagent finished with, and its context
//! is discarded as it stops, so a lost capture is gone. When the store is busy
//! past the budget the text is written here instead — one JSON file per capture,
//! under `<database parent>/hooks/spool/` beside the reminder files — and a
//! later open replays it through the same [`Store::passive_capture`] door the
//! hook would have used.
//!
//! The shape is one file per entry rather than one growing log because a drain
//! has to claim work without a lock of its own: a claim is a rename, which is
//! atomic on every platform this ships to, so two drainers take different
//! entries rather than both replaying the same one.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    Store,
    memory::{model::PassiveCapture, normalize},
};

/// How many entries the spool holds before the oldest are dropped.
///
/// Without a bound a store busy for a long time fills the directory, and each
/// entry is the whole text of a subagent's turn. Read by the tests beside
/// this, so the number that is applied is the number that is named.
pub const SPOOL_CAP: usize = 1_000;

/// Days after which an entry is dropped without being replayed.
///
/// Past this the capture is not worth keeping: a subagent's learnings belong to
/// the conversation that produced them, and replaying one a week later files it
/// into a store that has moved on.
pub const SPOOL_RETENTION_DAYS: i64 = 7;

const RETENTION_MILLIS: i64 = SPOOL_RETENTION_DAYS * 24 * 60 * 60 * 1_000;

/// Distinguishes two entries written in the same millisecond by one process.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The directory the spool lives in, beside the database.
///
/// The same `hooks/` the reminder state uses ([`super::nudge`]), so a store
/// keeps its side files in one place and a move takes them together.
pub(crate) fn directory(data_dir: &Path) -> PathBuf {
    data_dir.join("hooks").join("spool")
}

/// One capture waiting to be stored.
///
/// Plain fields rather than a nested [`PassiveCapture`], so the on-disk shape
/// does not move when the model type does and the timestamp can be read without
/// deserialising a subagent's whole turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    event: String,
    at: String,
    session_id: String,
    project: String,
    /// The directory the session belongs to, so a replay can ensure the session
    /// the busy-open path never got to create.
    directory: String,
    content: String,
    source: String,
}

impl Entry {
    fn capture(&self) -> PassiveCapture {
        PassiveCapture {
            session_id: self.session_id.clone(),
            project: self.project.clone(),
            content: self.content.clone(),
            source: self.source.clone(),
        }
    }
}

/// What the doctor reads: how many entries wait and how old the oldest is.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pending {
    pub entries: usize,
    pub oldest: Option<Duration>,
}

/// What one drain did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DrainReport {
    /// Entries this drain took by renaming. An entry it renamed back because
    /// the store was busy is counted here and may be claimed again by another
    /// drain, so across drains this can exceed the entries there were.
    pub claimed: usize,
    /// Claimed entries renamed back because the store was busy, for a later
    /// drain to take again. `claimed - returned` is what this drain replayed,
    /// and across drains it is what says no entry was replayed twice: the
    /// store keeps a replayed learning once whoever replays it, so `stored`
    /// cannot say it.
    pub returned: usize,
    /// Learnings that reached the store across those entries.
    pub stored: usize,
    /// Entries removed because replay failed for a reason a retry cannot mend.
    pub dropped: usize,
    /// Entries left behind, for the next drain.
    pub remaining: usize,
}

/// Writes one capture beside the database.
///
/// Only for a capture the store refused as busy: a replay of any other failure
/// would fail the same way, which is what the message beside this already says.
/// The content is redacted before it reaches the disk, so the `<private>`
/// promise holds for a file that outlives the process that wrote it.
///
/// The entry is written to a temporary file in the same directory and renamed
/// into place, so a drain running at the same time never sees half of a turn.
/// See [`temporary_name`] for why the temporary is not a name the drain reads.
pub(crate) fn spool(
    data_dir: &Path,
    event: &str,
    capture: &PassiveCapture,
    session_directory: &str,
) -> std::io::Result<()> {
    let dir = directory(data_dir);
    fs::create_dir_all(&dir)?;
    sweep(&dir, now_millis());

    let entry = Entry {
        event: event.to_owned(),
        at: chrono::Utc::now().to_rfc3339(),
        session_id: capture.session_id.clone(),
        project: capture.project.clone(),
        directory: session_directory.to_owned(),
        content: normalize::strip_private(&capture.content),
        source: capture.source.clone(),
    };
    let body = serde_json::to_vec(&entry).map_err(std::io::Error::other)?;
    let name = entry_name(now_millis());
    let temporary = dir.join(temporary_name(&name));
    fs::write(&temporary, &body)?;
    fs::rename(&temporary, dir.join(&name))?;

    enforce_cap(&dir);
    Ok(())
}

/// The name `spool` writes before renaming it into place.
///
/// It ends `.tmp` and not `.json`, because [`entries`] treats every `.json` name
/// as a capture: a temporary that ended in `.json` would be enumerated by a
/// concurrent drain, claimed, and read before the writer had finished it, which
/// is the half-a-turn read the rename exists to prevent.
pub(super) fn temporary_name(name: &str) -> String {
    format!("{name}.tmp")
}

/// Replays what a busy store left behind, oldest first.
///
/// Bounded by `deadline`, which for a hook is what is left of its own budget,
/// so the drain cannot outlast the agent that is waiting. The first busy error
/// stops the drain and leaves the rest for a later open: a busy store is the
/// one failure a retry mends, and replaying the next entry against it would
/// spend the same wait for the same answer. Any other error is one a retry
/// cannot mend, so that entry is counted as dropped and removed.
pub(crate) fn drain(store: &mut Store, deadline: Instant) -> DrainReport {
    let dir = directory(store.data_dir());
    let mut report = DrainReport::default();
    sweep(&dir, now_millis());
    for path in entries(&dir) {
        if Instant::now() >= deadline {
            break;
        }
        let Some(claimed) = claim(&path) else {
            // Another drainer renamed it out from under this one, which is the
            // whole point of claiming: it takes the entries this one does not.
            continue;
        };
        report.claimed += 1;
        let entry = match fs::read(&claimed)
            .ok()
            .and_then(|body| serde_json::from_slice::<Entry>(&body).ok())
        {
            Some(entry) => entry,
            None => {
                // A file that cannot be read is not a capture to keep waiting
                // for; it is litter, and keeping it would make `doctor` report
                // a spool that can never drain.
                let _ = fs::remove_file(&claimed);
                report.dropped += 1;
                continue;
            }
        };
        // The capture can name a session that does not exist yet: the busy-open
        // path spools before `ensure_session` could run, and a fresh store has
        // no session row at all. Ensure it from the entry, exactly as a live
        // `subagent-stop` does, so the replay stores the learnings instead of
        // failing `SessionNotFound` and deleting the subagent's only copy.
        match store.create_session(&entry.session_id, &entry.project, &entry.directory) {
            Ok(_) => {}
            Err(error) if error.is_busy() => {
                let _ = fs::rename(&claimed, &path);
                report.returned += 1;
                break;
            }
            Err(_) => {
                let _ = fs::remove_file(&claimed);
                report.dropped += 1;
                continue;
            }
        }
        match store.passive_capture(entry.capture()) {
            Ok(result) => {
                report.stored += result.saved;
                let _ = fs::remove_file(&claimed);
            }
            Err(failure) if failure.error.is_busy() => {
                // Put the name back, so the next drain finds the entry where it
                // left it rather than in a claimed file nothing looks at.
                let _ = fs::rename(&claimed, &path);
                report.returned += 1;
                break;
            }
            Err(_) => {
                let _ = fs::remove_file(&claimed);
                report.dropped += 1;
            }
        }
    }
    report.remaining = entries(&dir).len();
    report
}

/// The entries waiting in `dir`, oldest first, with the oldest entry's age.
pub(crate) fn pending(dir: &Path) -> Pending {
    let now = now_millis();
    let entries = entries(dir);
    let oldest = entries
        .iter()
        .filter_map(|path| entry_millis(path))
        .map(|written| Duration::from_millis(now.saturating_sub(written).max(0) as u64))
        .max();
    Pending {
        entries: entries.len(),
        oldest,
    }
}

/// An age in the largest unit that still reads as one.
///
/// The same shape the save reminder gives an age, and for the same
/// reason: "259200 seconds" is a number nobody converts, and the doctor line is
/// read by a person deciding whether to run the repair.
pub(crate) fn describe_age(age: Duration) -> String {
    let seconds = age.as_secs();
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

/// The entry files in `dir`, oldest first.
///
/// A claimed file is not one: it is a name this directory made up for an entry
/// already being replayed, and it ends in `.claimed-<pid>` rather than `.json`.
fn entries(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = read
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".json"))
        })
        .collect();
    found.sort();
    found
}

/// Removes entries past their retention, and the claimed litter beside them.
fn sweep(dir: &Path, now: i64) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        let age = entry_millis(&path)
            .map(|written| now.saturating_sub(written))
            .or_else(|| {
                path.metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|since| now.saturating_sub(since.as_millis() as i64))
            });
        if age.is_some_and(|age| age > RETENTION_MILLIS) {
            let _ = fs::remove_file(path);
        }
    }
}

/// Drops the oldest entries past the cap.
fn enforce_cap(dir: &Path) {
    let mut entries = entries(dir);
    if entries.len() <= SPOOL_CAP {
        return;
    }
    for path in entries.drain(..entries.len() - SPOOL_CAP) {
        let _ = fs::remove_file(path);
    }
}

/// Takes one entry out of every drainer's reach by renaming it.
fn claim(path: &Path) -> Option<PathBuf> {
    let mut name = path.file_name()?.to_os_string();
    name.push(format!(".claimed-{}", std::process::id()));
    let claimed = path.with_file_name(name);
    fs::rename(path, &claimed).ok()?;
    Some(claimed)
}

/// The wall clock the entry names are ordered by.
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or_default()
}

/// A name that sorts by when the entry was written.
///
/// The millisecond is zero-padded so a lexicographic sort is a chronological
/// one, and the process and counter make two entries written in the same
/// millisecond distinct.
fn entry_name(millis: i64) -> String {
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{millis:013}-{:010}-{counter:06}.json", std::process::id())
}

/// When the entry was written, read back out of its own name.
fn entry_millis(path: &Path) -> Option<i64> {
    path.file_name()?.to_str()?.split('-').next()?.parse().ok()
}
