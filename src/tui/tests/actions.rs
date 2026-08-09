//! Deleting things, behind a confirmation.

use super::*;

#[test]
fn a_delete_is_asked_about_before_it_reaches_the_store() {
    let (_temp, mut store) = store_of(3);
    let mut app = App::load(&mut store).unwrap();
    app.page = Page::Dashboard;
    let doomed = app.recent[0].id;

    // `d` does not delete. It asks the store what would go, and nothing is
    // pending until those numbers are in — a confirmation is worth having
    // only if what it says is what is about to happen.
    let action = app.handle_key(key(KeyCode::Char('d')));
    assert_eq!(
        action,
        Action::Confirm {
            what: Target::Observation(doomed),
            hard: false
        }
    );
    assert!(app.pending.is_none());
    apply_action(&mut app, &mut store, action);
    let pending = app.pending.clone().expect("a confirmation is up");
    assert_eq!(
        pending.action,
        Action::Delete {
            what: Target::Observation(doomed),
            hard: false
        }
    );
    assert_eq!(app.recent.len(), 3, "and nothing has gone yet");

    // Enter cancels, like every other key that is not a yes: the window
    // appears under somebody's hands, and Enter is the likeliest of those
    // to be already on its way down.
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Action::None);
    assert!(app.pending.is_none());
    assert_eq!(
        app.status.as_ref().map(|status| status.text.as_str()),
        Some("Cancelled")
    );

    // And a yes goes through.
    let action = app.handle_key(key(KeyCode::Char('D')));
    apply_action(&mut app, &mut store, action);
    let confirmed = app.handle_key(key(KeyCode::Char('y')));
    assert_eq!(
        confirmed,
        Action::Delete {
            what: Target::Observation(doomed),
            hard: true
        }
    );
    apply_action(&mut app, &mut store, confirmed);
    assert_eq!(
        app.recent.len(),
        2,
        "gone, and the list reloaded without it"
    );
}
