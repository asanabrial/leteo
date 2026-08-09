//! Break an invariant on purpose and report which tests noticed.
//!
//! Three times in one session a break-verification reported success without
//! having tested anything: `cargo fmt` had reformatted the line the patch
//! matched on, a two-word `cargo test` filter matched no tests at all, and a
//! guard passed because a fallback quietly supplied the right answer. Each
//! looked like proof.
//!
//! So the checking is mechanical. This asserts the mutation actually landed,
//! runs the whole suite rather than a filter, and fails loudly when nothing
//! breaks — a guard that survives its own invariant being removed is not a
//! guard.
//!
//! ```text
//! cargo run --manifest-path tools/mutate/Cargo.toml -- tools/guards.json
//! ```

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A mutation that makes the suite loop rather than fail is not a caught one.
///
/// Removing the progress check from the repository import does exactly that:
/// every round retries the same unappliable chunk and `cargo test` never
/// returns. It was counted as caught here for a while, which is the one thing
/// this tool exists to make impossible — the suite never reported on the
/// invariant, so nothing was shown to notice.
const SUITE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    file: String,
    #[serde(default)]
    old: Option<String>,
    #[serde(default)]
    new: Option<String>,
    /// Several edits that only mean anything together.
    #[serde(default)]
    edits: Vec<Edit>,
}

#[derive(Debug, Deserialize, Clone)]
struct Edit {
    old: String,
    new: String,
}

impl Case {
    fn edits(&self) -> Vec<Edit> {
        if !self.edits.is_empty() {
            return self.edits.clone();
        }
        match (&self.old, &self.new) {
            (Some(old), Some(new)) => vec![Edit {
                old: old.clone(),
                new: new.clone(),
            }],
            _ => Vec::new(),
        }
    }
}

/// What the file being mutated was, so a run that is killed leaves something
/// the next one repairs.
///
/// Both halves of that mattered. A `cargo test` racing a sweep reported four
/// failures that were not real, and a sweep killed between writing a mutation
/// and restoring it left the tree broken while the next one ran its whole list
/// against it — reporting its own damage as unrelated failures.
#[derive(Debug, Serialize, Deserialize)]
struct InFlight {
    file: String,
    original: String,
}

/// What a suite run produced.
enum Outcome {
    /// Passed, failed, ignored.
    Counts(usize, usize, usize),
    /// The suite never finished. Its own answer, not a failure count: a hung
    /// suite credited as a guard doing its job is the mistake this prevents.
    Timeout,
    /// It did not even build, with the output kept — "proves nothing" with no
    /// compiler text is a verdict nobody can act on.
    NoBuild(String),
}

fn repo() -> PathBuf {
    // `tools/mutate/` up to the repository root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two directories under the repository")
        .to_path_buf()
}

fn lock_path() -> PathBuf {
    repo().join("tools").join(".mutate-in-flight.json")
}

fn repair_if_interrupted() -> std::io::Result<()> {
    let lock = lock_path();
    if !lock.exists() {
        return Ok(());
    }
    let held: InFlight = serde_json::from_str(&std::fs::read_to_string(&lock)?)
        .expect("the in-flight file is written by this tool");
    let target = repo().join(&held.file);
    if std::fs::read_to_string(&target)? != held.original {
        write_exact(&target, &held.original)?;
        println!(
            "repaired {}: a previous run was interrupted mid-mutation",
            held.file
        );
    }
    std::fs::remove_file(lock)
}

/// Written byte for byte, with no newline translation.
///
/// The tree is CRLF on Windows and LF in the index. A writer that translated
/// would rewrite every line of the file it touched, and the restore afterwards
/// would leave a diff that has nothing to do with the mutation.
fn write_exact(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(contents.as_bytes())
}

/// One line of `test result: ok. 12 passed; 0 failed; 1 ignored`.
fn parse_totals(line: &str) -> Option<(usize, usize, usize)> {
    let rest = line.strip_prefix("test result: ")?;
    let rest = rest.split_once(". ")?.1;
    let mut passed = None;
    let mut failed = None;
    let mut ignored = None;
    // Skipped rather than bailed on, because the line ends `finished in 9.37s`
    // and `0 filtered out` — a parser that gave up on the first part with no
    // number in it read every suite as one that never built, and said the
    // baseline was not green with the build log underneath. It failed loudly,
    // which is the only reason this took a minute rather than a sweep.
    for part in rest.split(';') {
        let mut words = part.split_whitespace();
        let Some(Ok(count)) = words.next().map(str::parse::<usize>) else {
            continue;
        };
        match words.next() {
            Some("passed") => passed = Some(count),
            Some("failed") => failed = Some(count),
            Some("ignored") => ignored = Some(count),
            _ => {}
        }
    }
    Some((passed?, failed?, ignored?))
}

fn run_suite() -> (Outcome, Vec<String>) {
    let started = Instant::now();
    let mut child = match Command::new("cargo")
        .args(["test", "--no-fail-fast"])
        .current_dir(repo())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return (Outcome::NoBuild(error.to_string()), Vec::new()),
    };
    // Drained while it runs, on threads, and this is not a refinement.
    //
    // Waiting on the child while nothing reads its pipes deadlocks as soon as
    // `cargo test` writes more than the operating system will buffer: the child
    // blocks on the write, so it never exits, so the wait never returns, and
    // every case comes back as a timeout. Six did, including one whose mutation
    // — `require_sync_target` answering with an empty string — cannot make
    // anything loop. That is what gave it away.
    //
    // The first version of this port had one case in its smoke test that
    // actually reached `cargo`, and it happened to write little enough to fit.
    // A harness is worth what its own fixture reaches, like anything else here.
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let mut stderr = child.stderr.take().expect("stderr was piped");
    let reading = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buffer);
        buffer
    });
    let reading_errors = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr, &mut buffer);
        buffer
    });
    // Waited on rather than blocked in `output()`, because a mutation can make
    // the suite loop and the sweep has to survive that with a verdict.
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > SUITE_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return (
                    Outcome::Timeout,
                    vec!["<the suite did not finish; a test is looping>".to_owned()],
                );
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(200)),
            Err(error) => return (Outcome::NoBuild(error.to_string()), Vec::new()),
        }
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&reading.join().unwrap_or_default()),
        String::from_utf8_lossy(&reading_errors.join().unwrap_or_default())
    );
    let mut totals = (0, 0, 0);
    let mut saw_any = false;
    let mut failures = BTreeSet::new();
    for line in text.lines() {
        if let Some((passed, failed, ignored)) = parse_totals(line.trim_start()) {
            saw_any = true;
            totals = (totals.0 + passed, totals.1 + failed, totals.2 + ignored);
        }
        if let Some(rest) = line.trim_start().strip_prefix("test ")
            && let Some(name) = rest.strip_suffix(" ... FAILED")
        {
            failures.insert(name.to_owned());
        }
    }
    if !saw_any {
        return (Outcome::NoBuild(text), failures.into_iter().collect());
    }
    (
        Outcome::Counts(totals.0, totals.1, totals.2),
        failures.into_iter().collect(),
    )
}

fn main() -> std::process::ExitCode {
    let Some(list) = std::env::args().nth(1) else {
        eprintln!("usage: leteo-mutate <guards.json>");
        return std::process::ExitCode::from(2);
    };
    if let Err(error) = repair_if_interrupted() {
        eprintln!("could not repair an interrupted run: {error}");
        return std::process::ExitCode::from(2);
    }
    let cases: Vec<Case> = serde_json::from_str(
        &std::fs::read_to_string(&list).expect("the case list is readable"),
    )
    .expect("the case list is a JSON array of cases");

    let (baseline, failing) = run_suite();
    let (passing, ignored) = match baseline {
        Outcome::Counts(passed, 0, ignored) => (passed, ignored),
        // Which tests, or which error. "Not green" on its own is a verdict
        // nobody can act on — the same complaint this tool makes about a
        // compile error with no compiler output, and it was made about this
        // line first: the baseline came back red with nothing beside it and
        // the suite was green under the identical command a minute later.
        other => {
            println!("baseline is not green; fix that first");
            match other {
                Outcome::NoBuild(text) if text.trim().is_empty() => {
                    println!("  the suite produced no output at all");
                }
                Outcome::NoBuild(text) => {
                    let tail: String = text.chars().rev().take(2000).collect();
                    println!("{}", tail.chars().rev().collect::<String>());
                }
                Outcome::Timeout => println!("  and it did not finish"),
                Outcome::Counts(passed, failed, ignored) => {
                    println!("  {passed} passed, {failed} failed, {ignored} ignored");
                    for name in &failing {
                        println!("  - {name}");
                    }
                }
            }
            return std::process::ExitCode::from(1);
        }
    };
    println!("baseline: {passing} passing over {} cases", cases.len());
    // A test that did not run cannot catch anything, so SURVIVED means
    // "nothing that ran noticed" — not "nothing would".
    if ignored > 0 {
        println!(
            "WARNING: {ignored} test(s) ignored and not run; SURVIVED below means only that \
             nothing which ran noticed. CI covers these with TEST_DATABASE_URL and \
             `cargo test -- --ignored`."
        );
    }
    println!();

    let mut proved_nothing = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let started = Instant::now();
        let position = format!("[{}/{}]", index + 1, cases.len());
        let path = repo().join(&case.file);
        let original = std::fs::read_to_string(&path).expect("the case names a readable file");
        let edits = case.edits();
        // An `old` that appears more than once is not a case, it is a guess:
        // the first occurrence is mutated, silently, and usually not the one
        // meant. Five queries in `relations.rs` share a join clause, and a case
        // aimed at the caveat query hit an unrelated one and reported SURVIVED.
        if let Some(edit) = edits.iter().find(|edit| original.matches(&edit.old).count() > 1) {
            let matches = original.matches(&edit.old).count();
            println!(
                "  {position} {:<52} AMBIGUOUS - {matches} matches; add context",
                case.name
            );
            proved_nothing.push(case.name.clone());
            continue;
        }
        if edits.is_empty() || edits.iter().any(|edit| !original.contains(&edit.old)) {
            println!(
                "  {position} {:<52} NOT APPLIED - the text has moved",
                case.name
            );
            proved_nothing.push(case.name.clone());
            continue;
        }
        let mut mutated = original.clone();
        for edit in &edits {
            mutated = mutated.replacen(&edit.old, &edit.new, 1);
        }
        assert_ne!(mutated, original, "the mutation changed nothing");

        std::fs::write(
            lock_path(),
            serde_json::to_string(&InFlight {
                file: case.file.clone(),
                original: original.clone(),
            })
            .expect("the in-flight record serialises"),
        )
        .expect("the in-flight record is writable");
        write_exact(&path, &mutated).expect("the mutated file is writable");
        let (outcome, failures) = run_suite();
        write_exact(&path, &original).expect("the original is writable");
        let _ = std::fs::remove_file(lock_path());

        let verdict = match outcome {
            Outcome::Timeout => "TIMEOUT - case proves nothing".to_owned(),
            // Not a pass. A mutation that does not compile never ran, so it
            // says nothing about whether a guard would have caught it.
            Outcome::NoBuild(text) => {
                let directory = repo().join("tools").join(".mutate-failed-builds");
                let _ = std::fs::create_dir_all(&directory);
                let slug: String = case
                    .name
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '-' })
                    .take(60)
                    .collect();
                let _ = std::fs::write(
                    directory.join(format!("{:03}-{slug}.txt", index + 1)),
                    &text,
                );
                let first = text
                    .lines()
                    .find(|line| line.starts_with("error"))
                    .unwrap_or("<no error line in the output>");
                format!(
                    "COMPILE ERROR - case proves nothing: {}",
                    &first[..first.len().min(80)]
                )
            }
            Outcome::Counts(_, 0, _) => "SURVIVED".to_owned(),
            Outcome::Counts(_, failed, _) => format!("caught by {failed}"),
        };
        if verdict == "SURVIVED" || !verdict.starts_with("caught") {
            proved_nothing.push(case.name.clone());
        }
        println!(
            "  {position} {:<52} {verdict}  ({}s)",
            case.name,
            started.elapsed().as_secs()
        );
        for name in failures.iter().take(4) {
            println!("      {name}");
        }
    }

    println!();
    if proved_nothing.is_empty() {
        println!("every one of {} mutations was caught", cases.len());
        return std::process::ExitCode::SUCCESS;
    }
    println!("{} case(s) proved nothing:", proved_nothing.len());
    for name in &proved_nothing {
        println!("  - {name}");
    }
    std::process::ExitCode::from(1)
}
