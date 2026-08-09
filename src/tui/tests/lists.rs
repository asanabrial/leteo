//! Scrolling a list that is longer than the panel.

use super::*;

#[test]
fn selection_detail_timeline_and_back_transitions_are_consistent() {
    let mut app = test_app();
    app.recent.push(observation(2, "Second memory"));

    app.handle_key(key(KeyCode::Char('j')));
    assert_eq!(app.recent_selected, 1);
    assert_eq!(
        app.handle_key(key(KeyCode::Enter)),
        Action::OpenObservation(2)
    );

    app.open_detail(app.recent[1].clone(), Vec::new());
    assert_eq!(app.page, Page::Detail);
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Action::LoadTimeline(2));

    app.open_timeline(timeline(app.recent[1].clone()));
    assert_eq!(app.page, Page::Timeline);
    assert_eq!(app.selected_timeline_id(), Some(2));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.page, Page::Detail);
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.page, Page::Dashboard);
}

#[test]
fn a_search_shows_on_the_row_the_headings_and_the_counters() {
    let mut app = test_app();
    app.page = Page::Dashboard;
    app.query = "wizard".to_owned();
    // What a reload would have left behind: a few hits in a bigger store.
    app.stats.total_observations = 40;
    app.stats.total_prompts = 12;
    app.recent_total = 3;
    app.prompt_total = 0;
    app.prompts.clear();

    let mut terminal = Terminal::new(TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());

    assert!(
        has_query_row(&drawn),
        "the query row has to appear:\n{drawn}"
    );
    assert!(
        drawn.contains("/ wizard"),
        "and it has to say what was typed:\n{drawn}"
    );
    assert!(
        drawn.contains("Observations matching \"wizard\""),
        "and the heading has to stop claiming these are the recent ones:\n{drawn}"
    );
    // The counters become the answer to "where is it": three observations
    // matched, no prompts did, and that is readable before anything is read.
    assert!(drawn.contains("3 / 40"), "observation hits:\n{drawn}");
    assert!(drawn.contains("0 / 12"), "and no prompt hits:\n{drawn}");
    assert!(
        drawn.contains("FILTERS (1)"),
        "a search is a filter, and the panel counts it:\n{drawn}"
    );

    // And with nothing narrowed they say one number. `40 / 40` is a sum
    // somebody has to do to learn that no filter is in force.
    let mut plain = test_app();
    plain.page = Page::Dashboard;
    plain.stats.total_observations = 40;
    plain.recent_total = 40;
    terminal.draw(|frame| render(frame, &plain)).unwrap();
    let drawn = buffer_text(terminal.backend());
    assert!(!drawn.contains("40 / 40"), "{drawn}");
    assert!(drawn.contains("40"), "{drawn}");
}

#[test]
fn only_the_showing_list_is_on_screen() {
    // Three lists at once meant three short ones: the observations had four
    // rows on a forty-row terminal.
    let mut app = test_app();
    app.page = Page::Dashboard;
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

    for (list, present, absent) in [
        (ListKind::Observations, " Observations", " Sessions"),
        (ListKind::Sessions, " Sessions", " Prompts"),
        (ListKind::Prompts, " Prompts", " Observations"),
    ] {
        show_list(&mut app, list);
        terminal.draw(|frame| render(frame, &app)).unwrap();
        let drawn = buffer_text(terminal.backend());
        assert!(
            drawn.contains(present),
            "{list:?} should show:
{drawn}"
        );
        assert!(
            !drawn.contains(absent),
            "{list:?} should be the only list, saw {absent}:
{drawn}"
        );
    }
}

#[test]
fn the_marks_show_on_screen_and_the_lists_say_their_scope() {
    let mut app = test_app();
    app.page = Page::Dashboard;
    app.stats.projects = vec!["leteo".to_owned(), "engram".to_owned()];
    app.handle_key(key(KeyCode::Char('f')));
    app.handle_key(key(KeyCode::Char(' ')));

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    assert!(
        drawn.contains("[\u{2713}] leteo"),
        "a marked project has to look marked:\n{drawn}"
    );
    assert!(drawn.contains("[ ] engram"), "{drawn}");
    assert!(
        drawn.contains("FILTERS (1)"),
        "the heading says what is in force, because the marks scroll away:\n{drawn}"
    );
    assert!(
        drawn.contains("Observations in leteo"),
        "and the list names its scope:\n{drawn}"
    );

    // With several marked the title counts them rather than listing them
    // past the edge of the panel.
    app.handle_key(key(KeyCode::Char('j')));
    app.handle_key(key(KeyCode::Char(' ')));
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    assert!(drawn.contains("in 2 projects"), "{drawn}");
}

#[test]
fn the_filter_panel_grows_enough_to_be_usable() {
    // Five rows fits four numbers and three projects. A store with sixteen
    // showed three of them, which is a list you cannot work with.
    let mut app = test_app();
    app.page = Page::Dashboard;
    app.stats.projects = (1..=16).map(|n| format!("project-{n:02}")).collect();

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let visible = |drawn: &str| {
        (1..=16)
            .filter(|n| drawn.contains(&format!("project-{n:02}")))
            .count()
    };
    let resting = buffer_text(terminal.backend());
    assert!(
        visible(&resting) <= 4,
        "the resting band is small on purpose, saw {}",
        visible(&resting)
    );

    app.handle_key(key(KeyCode::Char('f')));
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let opened = buffer_text(terminal.backend());
    assert!(
        visible(&opened) >= 12,
        "opened, it has to show most of them, saw {}",
        visible(&opened)
    );
    // And the list below survives the squeeze rather than vanishing.
    assert!(opened.contains("Observations"), "{opened}");
}

#[test]
fn enter_reads_the_selected_prompt() {
    // The dashboard counted prompts and showed none of them. The panel gives
    // one line each, so opening one is how the whole thing gets read.
    let mut app = test_app();
    app.page = Page::Dashboard;
    show_list(&mut app, ListKind::Prompts);
    assert_eq!(app.handle_key(key(KeyCode::Enter)), Action::None);
    let status = app.status.clone().expect("Enter should report the prompt");
    assert!(
        status.text.contains("why does the wizard look plain?"),
        "{}",
        status.text
    );
    assert!(!status.is_error);
}

#[test]
fn a_narrow_window_stacks_rather_than_squeezing_the_pair() {
    // Wide enough for the drawing but not for the drawing and the menu side
    // by side. Overlapping them would put the labels on top of the cat.
    let mut app = test_app();
    app.page = Page::Home;
    let mut terminal = Terminal::new(TestBackend::new(50, 44)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    for (label, _) in MENU {
        let label = label(crate::i18n::screens(crate::settings::Interface::English));
        assert!(drawn.contains(label), "missing {label}:\n{drawn}");
    }
    // Every menu row has to be a row of its own, with nothing else on it.
    for line in drawn.lines() {
        if line.contains((MENU[0].0)(crate::i18n::screens(
            crate::settings::Interface::English,
        ))) {
            assert!(
                !line.chars().any(|c| ('\u{2801}'..='\u{28FF}').contains(&c)),
                "a label is sharing its row with the drawing: {line:?}"
            );
        }
    }
}

#[test]
fn a_search_narrows_all_three_lists_at_once() {
    // The point of folding search into the dashboard: one query, and every
    // list answers it. The old search page could only ever show
    // observations, so "which session was that in" had no answer at all.
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("tui.db"))).unwrap();
    for (session, project, word) in [("s1", "leteo", "postgres"), ("s2", "engram", "sqlite")] {
        store.create_session(session, project, "C:/repo").unwrap();
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: session.to_owned(),
                kind: "decision".to_owned(),
                title: format!("Chose {word}"),
                content: format!("we went with {word}"),
                tool_name: None,
                project: Some(project.to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
        store
            .add_prompt(crate::memory::model::AddPrompt {
                session_id: session.to_owned(),
                content: format!("why {word}?"),
                project: Some(project.to_owned()),
            })
            .unwrap();
    }

    let mut app = App::load(&mut store).unwrap();
    app.query = "postgres".to_owned();
    app.refresh(&mut store).unwrap();

    assert_eq!(app.recent.len(), 1, "one observation says postgres");
    assert_eq!(app.prompts.len(), 1, "and one prompt asks about it");
    assert_eq!(
        app.sessions.len(),
        1,
        "and the session it happened in comes back with them"
    );
    assert_eq!(app.sessions[0].id, "s1");

    // The project filter composes with it rather than replacing it: asking
    // for postgres inside engram is a question with no answer, and the
    // screen has to say so rather than quietly showing one of the two.
    app.projects_filter = vec!["engram".to_owned()];
    app.refresh(&mut store).unwrap();
    assert!(app.recent.is_empty(), "{:?}", app.recent);
    assert!(app.prompts.is_empty());
    assert!(app.sessions.is_empty());

    // And clearing the query puts the recent lists back, still inside the
    // project that is still ticked.
    app.query.clear();
    app.refresh(&mut store).unwrap();
    assert_eq!(app.recent.len(), 1);
    assert_eq!(app.recent[0].title, "Chose sqlite");
}

#[test]
fn the_cursor_moves_inside_the_window_without_asking_the_store() {
    // The window is read four hundred rows at a time so that scrolling is a
    // cursor moving. A query per keypress is what this guards against.
    let (mut selected, mut offset) = (5, 0);
    assert!(!App::step(&mut selected, &mut offset, WINDOW, 3313, 28, 1));
    assert_eq!((selected, offset), (6, 0));

    // A screenful, still inside it.
    assert!(!App::step(&mut selected, &mut offset, WINDOW, 3313, 28, 28));
    assert_eq!((selected, offset), (34, 0));
    assert!(!App::step(
        &mut selected,
        &mut offset,
        WINDOW,
        3313,
        28,
        -28
    ));
    assert_eq!((selected, offset), (6, 0));
}

#[test]
fn walking_out_of_the_window_slides_it_without_the_screen_jumping() {
    // Crossing has to be invisible going down: the row moved to lands on the
    // last visible row, exactly where the next row would have appeared, so
    // the panel shifts by the one row it was going to shift by anyway.
    let height = 28;
    let (mut selected, mut offset) = (WINDOW - 1, 0);
    assert!(App::step(
        &mut selected,
        &mut offset,
        WINDOW,
        3313,
        height,
        1
    ));
    assert_eq!(offset + selected, WINDOW, "one row on, as asked");
    assert_eq!(selected, height - 1, "and on the last visible row");
    assert!(
        WINDOW - selected > height,
        "with the rest of the window read out ahead, so the next row down is \
             not another question"
    );

    // Going back up slides it the other way, and leaves its room above
    // rather than below.
    let (mut selected, mut offset) = (0, 500);
    assert!(App::step(
        &mut selected,
        &mut offset,
        WINDOW,
        3313,
        height,
        -1
    ));
    assert_eq!(offset + selected, 499);
    assert!(offset < 499, "room above it now");
    assert!(
        selected > height,
        "and not another question on the next press"
    );
}

#[test]
fn a_window_past_the_end_steps_back_rather_than_showing_nothing() {
    // Deleting the last row, or narrowing until the list is shorter than
    // where somebody had scrolled to, leaves the offset pointing past
    // everything — and an empty window reads as "nothing found".
    let (_temp, mut store) = store_of(10);
    let mut app = App::load(&mut store).unwrap();
    app.page = Page::Dashboard;
    app.recent_offset = 500;

    app.refresh(&mut store).unwrap();

    assert_eq!(app.recent_offset, 0, "stepped back to rows that exist");
    assert_eq!(app.recent.len(), 10);
}

#[test]
fn the_frame_says_which_row_the_cursor_is_on() {
    let (_temp, mut store) = store_of(60);
    let mut app = App::load(&mut store).unwrap();
    app.page = Page::Dashboard;

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    assert!(drawn.contains("1 of 60"), "{drawn}");

    app.handle_key(key(KeyCode::End));
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    assert!(drawn.contains("60 of 60"), "{drawn}");
    // The span of rows read is not what the corner reports: the store is
    // read four hundred at a time and the panel shows thirty, so a corner
    // saying `1-400 of 60` would contradict what is on screen.
    assert!(!drawn.contains('\u{2013}'), "{drawn}");

    // A list that fits says nothing: the answer is already visible.
    let plain = test_app();
    terminal.draw(|frame| render(frame, &plain)).unwrap();
    assert!(!buffer_text(terminal.backend()).contains(" of "));
}

#[test]
fn deleting_a_prompt_is_aimed_at_the_prompt_under_the_cursor() {
    let (_temp, mut store) = store_of(1);
    for content in ["first question", "second question"] {
        store
            .add_prompt(crate::memory::model::AddPrompt {
                session_id: "s1".to_owned(),
                content: content.to_owned(),
                project: Some("leteo".to_owned()),
            })
            .unwrap();
    }
    let mut app = App::load(&mut store).unwrap();
    app.page = Page::Dashboard;
    app.list = ListKind::Prompts;
    let doomed = app.prompts[0].id;

    let action = app.handle_key(key(KeyCode::Char('d')));
    assert_eq!(
        action,
        Action::Confirm {
            what: Target::Prompt(doomed),
            hard: false
        }
    );
    apply_action(&mut app, &mut store, action);
    let confirmed = app.handle_key(key(KeyCode::Char('y')));
    apply_action(&mut app, &mut store, confirmed);

    assert_eq!(app.prompts.len(), 1);
    assert_eq!(app.recent.len(), 1, "and the memory is untouched");
}

#[test]
fn the_confirmation_is_a_window_over_the_screen() {
    let (_temp, mut store) = store_of(3);
    let mut app = App::load(&mut store).unwrap();
    app.page = Page::Dashboard;
    let action = app.handle_key(key(KeyCode::Char('D')));
    apply_action(&mut app, &mut store, action);

    let mut terminal = Terminal::new(TestBackend::new(90, 20)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());

    assert!(drawn.contains("Delete memory #"), "{drawn}");
    assert!(drawn.contains("This cannot be undone."), "{drawn}");
    assert!(
        drawn.contains("y  delete"),
        "the window names the answers:\n{drawn}"
    );
    // And the footer stops offering keys that do not work while it is up.
    assert!(!drawn.contains("Tab next"), "{drawn}");

    // Nothing shows through it: a half-legible list under a warning reads as
    // a rendering fault and takes attention off the one thing being asked.
    let window_row = drawn
        .lines()
        .find(|line| line.contains("This cannot be undone."))
        .expect("the warning is on screen");
    assert!(
        !window_row.contains("Memory 00"),
        "a list row is showing through the window:\n{drawn}"
    );
}

#[test]
fn copying_yields_the_selected_memory_and_installing_asks_first() {
    let mut app = test_app();

    let Action::Copy(text) = app.handle_key(key(KeyCode::Char('y'))) else {
        panic!("y copies the selected memory");
    };
    assert!(text.starts_with("First memory"));
    assert!(text.contains("A detailed observation body"));

    // The sessions list has no observation under the cursor, and the
    // observation cursor is still resting on a row nobody can see.
    app.handle_key(key(KeyCode::Char('s')));
    assert_eq!(
        app.handle_key(key(KeyCode::Char('y'))),
        Action::None,
        "there is nothing to copy while the sessions list is showing"
    );
    app.handle_key(key(KeyCode::Char('r')));

    app.handle_key(key(KeyCode::Char('S')));
    assert_eq!(app.page, Page::Setup);
    assert!(
        app.wizard.is_some(),
        "opening setup should start the wizard"
    );
}
