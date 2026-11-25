//! Searching and filtering from the dashboard.

use super::*;

#[test]
fn typing_a_query_searches_as_it_goes_and_keeps_q_as_text() {
    let mut app = test_app();

    assert_eq!(app.handle_key(key(KeyCode::Char('/'))), Action::None);
    assert_eq!(app.page, Page::Dashboard);
    assert_eq!(app.focus, Focus::Query);

    // Every letter applies, the way ticking a project does. One narrowing
    // taking effect at once and the other waiting for Enter was two things
    // doing the same job behaving differently on the same screen.
    //
    // The `q` in "query" must not quit the program, which is why the input
    // takes every key while it has them.
    for character in ['q', 'u', 'e', 'r', 'y'] {
        assert_eq!(
            app.handle_key(key(KeyCode::Char(character))),
            Action::Narrow
        );
    }
    assert_eq!(app.query, "query");
    assert_eq!(app.handle_key(key(KeyCode::Backspace)), Action::Narrow);
    assert_eq!(app.query, "quer");

    // Enter has nothing left to run: it steps out of the input, and so does
    // Esc. Neither undoes the search — it has been in force since the first
    // letter, and dropping it is what Esc does from the list.
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Action::None);
    assert_eq!(app.focus, Focus::List);
    assert_eq!(app.query, "quer", "and what was typed stays applied");
}

#[test]
fn a_search_matches_the_word_that_is_still_being_typed() {
    // Without this every partial word finds nothing, and a search that runs
    // as it is typed spends most of its life saying the store is empty.
    let (_temp, mut store) = store_of(0);
    for title in ["Chose postgres", "Chose sqlite"] {
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: title.to_owned(),
                content: "body".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    let mut app = App::load(&mut store).unwrap();
    app.page = Page::Dashboard;
    app.handle_key(key(KeyCode::Char('/')));

    for (character, expected) in [('p', 1), ('o', 1), ('s', 1), ('t', 1)] {
        let action = app.handle_key(key(KeyCode::Char(character)));
        apply_action(&mut app, &mut store, action);
        assert_eq!(
            app.recent_total, expected,
            "after typing {:?} the list should hold {expected}",
            app.query
        );
    }

    // And the word before the one being typed is a finished word: matching
    // it by prefix too would widen a search that had been narrowed.
    for character in [' ', 's', 'q'] {
        let action = app.handle_key(key(KeyCode::Char(character)));
        apply_action(&mut app, &mut store, action);
    }
    assert_eq!(app.recent_total, 0, "\"post sq\" is in neither of them");
}

#[test]
fn there_is_nothing_to_search_in_an_empty_store() {
    // The empty dashboard is the cat and a sentence, with no panels and no
    // row to type on — so `/` there would put the keys in an input that is
    // not drawn, and the screen would stop responding.
    let mut app = test_app();
    app.stats.total_observations = 0;
    app.recent.clear();
    app.sessions.clear();

    app.handle_key(key(KeyCode::Char('/')));
    assert_eq!(app.focus, Focus::List);
    assert!(
        app.status.as_ref().is_some_and(|status| status.is_error),
        "and it says why rather than doing nothing: {:?}",
        app.status
    );
}

#[test]
fn f_steps_into_the_filter_panel_and_out_again() {
    let mut app = test_app();
    app.page = Page::Dashboard;

    app.handle_key(key(KeyCode::Char('f')));
    assert_eq!(app.focus, Focus::Filters);
    app.handle_key(key(KeyCode::Char('f')));
    assert_eq!(app.focus, Focus::List, "f is a door, not a one-way trip");

    // Esc leaves the mode before it leaves the page: it should undo the
    // smallest thing it can.
    app.handle_key(key(KeyCode::Char('f')));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus, Focus::List);
    assert_eq!(app.page, Page::Dashboard, "Esc should not also leave here");

    // And Tab out of the filter shows the next list rather than staying
    // in a panel somebody has finished with.
    app.handle_key(key(KeyCode::Char('f')));
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::List);
    assert_eq!(app.list, ListKind::Sessions);
}

#[test]
fn space_marks_and_unmarks_projects() {
    let mut app = test_app();
    app.page = Page::Dashboard;
    app.stats.projects = vec!["leteo".to_owned(), "engram".to_owned()];
    app.handle_key(key(KeyCode::Char('f')));

    // Several at once: asking about two projects together is a question
    // somebody has, and the store answers it in one query.
    assert_eq!(app.handle_key(key(KeyCode::Char(' '))), Action::Refresh);
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(app.projects_filter, vec!["leteo", "engram"]);

    // Unticking is how a filter is cleared, rather than a key nobody would
    // guess at.
    app.handle_key(key(KeyCode::Char(' ')));
    assert_eq!(app.projects_filter, vec!["leteo"]);
    app.handle_key(key(KeyCode::Char('k')));
    app.handle_key(key(KeyCode::Char(' ')));
    assert!(
        app.projects_filter.is_empty(),
        "no marks means every project"
    );
}

#[test]
fn deleting_a_memory_removes_it_from_the_open_search_results() {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("tui.db"))).unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    let doomed = store
        .add_observation(crate::memory::model::AddObservation {
            session_id: "s1".to_owned(),
            kind: "decision".to_owned(),
            title: "Doomed memory".to_owned(),
            content: "searchneedle".to_owned(),
            tool_name: None,
            project: Some("leteo".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap()
        .observation;
    let mut app = App::load(&mut store).unwrap();
    app.page = Page::Dashboard;
    app.query = "searchneedle".to_owned();
    app.refresh(&mut store).unwrap();
    assert_eq!(app.recent.len(), 1, "the search found it");

    let quit = apply_action(
        &mut app,
        &mut store,
        Action::Delete {
            what: Target::Observation(doomed.id),
            hard: false,
        },
    );

    assert!(!quit);
    // The search page used to keep its own copy of the results, which a
    // refresh did not touch — so a deleted memory stayed on screen and
    // opening it failed with "observation not found". There is one list
    // now, and one reload that produces it.
    assert!(
        app.recent.is_empty(),
        "a deleted memory must not stay on screen"
    );
    assert_eq!(app.recent_selected, 0);
    assert!(
        app.status.as_ref().is_some_and(|status| !status.is_error),
        "{:?}",
        app.status
    );
}

#[test]
fn deleting_a_project_takes_its_sessions_memories_and_prompts() {
    let (_temp, mut store) = store_of(3);
    // A second project, to show the delete is aimed rather than general.
    store.create_session("s2", "engram", "C:/other").unwrap();
    store
        .add_observation(crate::memory::model::AddObservation {
            session_id: "s2".to_owned(),
            kind: "decision".to_owned(),
            title: "Elsewhere".to_owned(),
            content: "body".to_owned(),
            tool_name: None,
            project: Some("engram".to_owned()),
            scope: "project".to_owned(),
            topic_key: None,
            prompt_sync_id: None,
        })
        .unwrap();
    let mut app = App::load(&mut store).unwrap();
    app.page = Page::Dashboard;

    // In the filter panel, `d` is aimed at the project whose box is
    // highlighted, so the key means the same thing wherever it is pressed.
    app.handle_key(key(KeyCode::Char('f')));
    let at = app
        .stats
        .projects
        .iter()
        .position(|project| project == "leteo")
        .expect("the store has it");
    app.project_selected = at;

    let action = app.handle_key(key(KeyCode::Char('D')));
    assert_eq!(
        action,
        Action::Confirm {
            what: Target::Project("leteo".to_owned()),
            hard: true
        }
    );
    apply_action(&mut app, &mut store, action);
    let pending = app.pending.clone().expect("a confirmation is up");
    assert!(pending.heading.contains("Delete project leteo"));
    assert!(
        pending.detail.iter().any(|line| line == "3 memory(s)"),
        "{:?}",
        pending.detail
    );
    assert!(pending.detail.iter().any(|line| line == "1 session(s)"));

    let confirmed = app.handle_key(key(KeyCode::Char('y')));
    apply_action(&mut app, &mut store, confirmed);
    assert_eq!(
        app.stats.projects,
        vec!["engram".to_owned()],
        "the project went, and the other one did not"
    );
    assert_eq!(app.recent.len(), 1);
    assert_eq!(app.recent[0].title, "Elsewhere");
}

/// A paste is one thing somebody copied, and none of it is a command.
///
/// A terminal that does not support bracketed paste delivers a paste as the
/// keystrokes it was copied from, which is how a bug report pasted into the
/// dashboard deleted the memory under the cursor: the first newline was Enter,
/// which steps out of the query line, then `d` raised the delete window and `y`
/// answered it.
#[test]
fn a_pasted_question_is_text_and_never_a_command() {
    let (_temp, mut store) = store_of(1);
    let mut app = test_app();
    assert_eq!(app.focus, Focus::List);

    let burst = [
        Event::Key(key(KeyCode::Char('m'))),
        Event::Key(key(KeyCode::Char('e'))),
        Event::Key(key(KeyCode::Char('m'))),
        Event::Key(key(KeyCode::Enter)),
        Event::Key(key(KeyCode::Char('d'))),
        Event::Key(key(KeyCode::Char('y'))),
    ];
    let delivered = delivery(burst[0].clone(), burst[1..].to_vec());
    assert!(
        matches!(delivered, Delivery::Paste(_)),
        "the burst is one paste"
    );
    assert!(!deliver(&mut app, &mut store, delivered));

    assert_eq!(
        store.count_observations(None).unwrap(),
        1,
        "the memory under the cursor is still there"
    );
    assert_eq!(app.query, "mem dy");
    assert_eq!(
        app.focus,
        Focus::Query,
        "the words went into the query line"
    );
    assert!(
        app.pending.is_none(),
        "nothing in a paste raises a confirmation"
    );
}

/// One key with nothing behind it is still a key, or the dashboard would have
/// no commands left.
#[test]
fn a_lone_key_is_still_a_command() {
    let (_temp, mut store) = store_of(1);
    let mut app = test_app();

    assert!(!deliver(
        &mut app,
        &mut store,
        delivery(Event::Key(key(KeyCode::Char('d'))), Vec::new())
    ));
    let pending = app
        .pending
        .as_ref()
        .expect("the delete went up for agreement");
    assert_eq!(
        pending.action,
        Action::Delete {
            what: Target::Observation(1),
            hard: false
        }
    );
    assert!(app.query.is_empty());
}

/// A press with its own release behind it, a held key's repeat, or a control
/// chord beside a character, is still the key it is: none of them is text a
/// paste would have carried.
#[test]
fn a_release_or_repeat_does_not_turn_a_press_into_text() {
    let press = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE);
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('d'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    let repeat =
        KeyEvent::new_with_kind(KeyCode::Char('d'), KeyModifiers::NONE, KeyEventKind::Repeat);
    let chord = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);

    assert_eq!(
        delivery(Event::Key(press), vec![Event::Key(release)]),
        Delivery::Keys(vec![press])
    );
    assert_eq!(
        delivery(Event::Key(press), vec![Event::Key(repeat)]),
        Delivery::Keys(vec![press, repeat])
    );
    assert_eq!(
        delivery(Event::Key(chord), vec![Event::Key(press)]),
        Delivery::Keys(vec![chord, press])
    );
}

/// A burst with no text in it is every key it holds: a held arrow moves by as
/// many steps as the terminal sent, rather than losing all but the first.
#[test]
fn a_burst_of_arrows_moves_by_every_key_in_it() {
    let (_temp, mut store) = store_of(4);
    let mut app = App::load(&mut store).unwrap();
    app.navigate(Page::Dashboard);
    assert_eq!(app.recent_selected, 0);

    let down = Event::Key(key(KeyCode::Down));
    assert!(!deliver(
        &mut app,
        &mut store,
        delivery(down.clone(), vec![down])
    ));
    assert_eq!(app.recent_selected, 2);
}

/// The bracketed form is what a terminal that supports the mode sends: one
/// event carrying the newlines, which fold rather than switching focus.
#[test]
fn a_bracketed_paste_keeps_the_words_and_folds_the_newlines() {
    let (_temp, mut store) = store_of(1);
    let mut app = test_app();

    assert!(!deliver(
        &mut app,
        &mut store,
        delivery(
            Event::Paste("first line\r\nsecond line".to_owned()),
            Vec::new()
        )
    ));
    assert_eq!(app.query, "first line second line");
    assert_eq!(app.focus, Focus::Query);
    assert!(app.pending.is_none());
}

/// The confirmation is not merely spared the paste: a `y` that arrives with the
/// `d` that would raise it is text, because the delivery is one paste and never
/// a list of keys.
#[test]
fn a_confirming_key_in_the_same_burst_as_the_delete_raises_nothing() {
    let (_temp, mut store) = store_of(1);
    let mut app = test_app();

    assert!(!deliver(
        &mut app,
        &mut store,
        delivery(
            Event::Key(key(KeyCode::Char('d'))),
            vec![Event::Key(key(KeyCode::Char('y')))]
        )
    ));
    assert_eq!(app.query, "dy");
    assert!(
        app.pending.is_none(),
        "the delete never went up for agreement"
    );
    assert_eq!(app.focus, Focus::Query);
}

/// A tab is a word boundary in pasted text, the same as a line break.
#[test]
fn a_pasted_tab_separates_words_instead_of_disappearing() {
    let (_temp, mut store) = store_of(1);
    let mut app = test_app();

    deliver(
        &mut app,
        &mut store,
        delivery(Event::Paste("first\tsecond".to_owned()), Vec::new()),
    );
    assert_eq!(app.query, "first second");
}

/// A paste joins what is already typed, trims its ends, and types nothing when
/// there is nothing but whitespace in it.
#[test]
fn a_paste_joins_one_line_to_what_was_already_typed() {
    let mut app = test_app();
    app.handle_key(key(KeyCode::Char('/')));
    app.handle_key(key(KeyCode::Char('f')));
    app.handle_key(key(KeyCode::Char('i')));

    assert_eq!(app.paste(" \r\n x\r\n "), Action::Narrow);
    assert_eq!(app.query, "fi x");
    assert_eq!(app.paste(" \r\n\t "), Action::None, "nothing to type");
    assert_eq!(app.query, "fi x");
}

/// The setup flow and the detail pages have no query row, so a paste there has
/// nowhere to go and changes nothing.
#[test]
fn a_paste_on_a_page_without_a_query_line_is_dropped() {
    let (_temp, mut store) = store_of(1);
    let mut app = test_app();
    app.navigate(Page::Help);

    assert!(!deliver(
        &mut app,
        &mut store,
        delivery(Event::Paste("deleted by accident".to_owned()), Vec::new())
    ));
    assert!(app.query.is_empty());
    assert_eq!(app.page, Page::Help, "the paste did not move the screen");
}
