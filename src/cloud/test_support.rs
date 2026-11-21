//! What the PostgreSQL-backed tests share.

use std::sync::atomic::{AtomicU64, Ordering};

/// A name suffix no other test, thread or process sharing the database can
/// also draw.
///
/// The tests used to take `Utc::now().timestamp_micros()`. They run as threads
/// of one process, started together, so two of them can read the same
/// microsecond; they then minted principals with the same display name, and
/// `SignedInAdmin::revoke` finds its principal by that name, so one test
/// revoking its session signed the other out half-way through. The victim saw
/// a 303 to the sign-in page where it expected a 400, and passed on rerun.
///
/// Digits only, at fixed widths: one test builds a search word from it, and a
/// separator would make the word several terms to the full-text parser.
pub(crate) fn unique_stamp() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}{:010}{:06}",
        chrono::Utc::now().timestamp_micros(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::unique_stamp;
    use std::collections::HashSet;
    use std::sync::{Arc, Barrier};

    #[test]
    fn concurrent_tests_never_draw_the_same_stamp() {
        const THREADS: usize = 16;
        const ROUNDS: usize = 50;
        let mut drawn: Vec<String> = Vec::new();
        for _ in 0..ROUNDS {
            let start = Arc::new(Barrier::new(THREADS));
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    let start = Arc::clone(&start);
                    std::thread::spawn(move || {
                        start.wait();
                        unique_stamp()
                    })
                })
                .collect();
            drawn.extend(handles.into_iter().map(|handle| handle.join().unwrap()));
        }
        let distinct: HashSet<&String> = drawn.iter().collect();
        assert_eq!(distinct.len(), THREADS * ROUNDS);
    }
}
