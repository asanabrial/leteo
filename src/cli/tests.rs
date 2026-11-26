//! What `doctor --repair` does when it is asked about one check.

use super::Repairs;
use crate::memory::model::DoctorCheck;

/// Every check the doctor can report says which repairs answer it.
///
/// `for_check` is a match over string codes, so a check added to `CODES`
/// without a row here answers `None` and `--check <that> --repair` would repair
/// nothing while the command still exited successfully.
#[test]
fn every_doctor_check_names_the_repairs_it_has() {
    for code in DoctorCheck::CODES {
        assert!(
            Repairs::for_check(code).is_some(),
            "{code} has no row in Repairs::for_check"
        );
    }
    assert!(
        Repairs::for_check("not_a_check").is_none(),
        "an unknown code is not a check with no repairs"
    );
}

/// One check's repairs are its own and drag no others in.
///
/// The bug this table exists for: `--check hook_spool --repair` rebuilt three
/// full-text indexes to answer a question about a directory of capture files.
#[test]
fn a_check_repairs_only_what_it_names() {
    let spool = Repairs::for_check("hook_spool").expect("the spool check");
    assert_eq!(
        spool,
        Repairs {
            spool: true,
            ..Repairs::default()
        }
    );
    assert!(
        !spool.indexes,
        "asking about the spool must not rebuild an index"
    );

    // Re-stemming writes the rows the Snowball index is built from, so the
    // index is rebuilt with it — one repair, two steps, and the pair is what
    // actually mends the check.
    let language = Repairs::for_check("memory_language").expect("the language check");
    assert!(language.language && language.indexes, "{language:?}");

    // The model and the vectors it leaves behind are the same repair read from
    // two checks.
    assert_eq!(
        Repairs::for_check("semantic_model"),
        Repairs::for_check("semantic_vectors")
    );

    // A finding no repair can mend names none of them, rather than being
    // mistaken for one that repairs everything.
    for code in [
        "busy_timeout",
        "journal_mode",
        "settings_readable",
        "topic_key_uniqueness",
        "supersession_cycles",
        "sqlite_integrity",
        "foreign_keys",
        DoctorCheck::STALE_CITATIONS,
    ] {
        assert_eq!(
            Repairs::for_check(code),
            Some(Repairs::default()),
            "{code} is mended by something"
        );
    }
}
