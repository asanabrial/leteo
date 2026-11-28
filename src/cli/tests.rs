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

/// A prefix two tokens share is refused by count, not guessed at.
///
/// The prefix is four random bytes with no uniqueness in the table, so two
/// tokens can carry one. Revoking the wrong one of two is how a leaked
/// credential survives the incident it was meant to end; the token itself and
/// the id are the two ways to say which one.
#[cfg(feature = "cloud-server")]
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to an isolated PostgreSQL database"]
async fn an_ambiguous_token_prefix_is_refused_and_the_token_or_id_is_not() {
    const PEPPER: &str = "a-token-pepper-of-at-least-32-bytes-long";
    let store = crate::cloud::CloudStore::connect(&std::env::var("TEST_DATABASE_URL").unwrap(), 2)
        .await
        .unwrap();
    let stamp = format!(
        "{}{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let principal = store
        .create_principal("human", &format!("ambiguous-{stamp}"), "admin")
        .await
        .unwrap();
    let hasher = crate::cloud::ManagedTokenHasher::new(PEPPER).unwrap();
    let first = crate::cloud::ManagedToken::generate("test");
    let twin = crate::cloud::ManagedToken {
        raw: format!("{}-twin", first.raw),
        prefix: first.prefix.clone(),
    };
    for token in [&first, &twin] {
        let verifier = hasher.hash(&token.raw).unwrap();
        store
            .store_managed_token(principal, token, &verifier, "twin")
            .await
            .unwrap();
    }
    let ids = store
        .find_managed_token_ids_by_prefix(&first.prefix)
        .await
        .unwrap();
    assert_eq!(ids.len(), 2, "the fixture has two tokens under one prefix");

    let refused = super::resolve_token_id(&store, PEPPER, &first.prefix)
        .await
        .expect_err("two tokens are two answers");
    assert!(
        refused.to_string().contains("2 managed tokens"),
        "{refused}"
    );

    assert_eq!(
        super::resolve_token_id(&store, PEPPER, &first.raw)
            .await
            .unwrap(),
        ids[0],
        "the token itself says which of the two, without being echoed"
    );
    assert_eq!(
        super::resolve_token_id(&store, PEPPER, &ids[0].to_string())
            .await
            .unwrap(),
        ids[0],
        "a numeric id is taken as given, not looked up as a prefix"
    );
    let absent = super::resolve_token_id(&store, PEPPER, &format!("ltc_test_absent{stamp}"))
        .await
        .expect_err("a prefix nobody holds names no token");
    assert!(absent.to_string().contains("no managed token"), "{absent}");
    assert!(
        !absent.to_string().contains("absent"),
        "the value an operator typed is not echoed back: {absent}"
    );

    sqlx::query("DELETE FROM cloud_principals WHERE id = $1")
        .bind(principal)
        .execute(store.pool())
        .await
        .unwrap();
}
