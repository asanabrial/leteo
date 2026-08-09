//! Moving between pages, and what a key means where.

use super::*;

#[test]
fn slash_reaches_the_dashboard_from_wherever_it_is_pressed() {
    // `/` is offered on the home screen's footer, and a key that quietly did
    // nothing on half the pages would read as broken.
    let mut app = test_app();
    app.page = Page::Help;

    app.handle_key(key(KeyCode::Char('/')));
    assert_eq!(app.page, Page::Dashboard);
    assert_eq!(app.focus, Focus::Query);
}

#[test]
fn esc_undoes_the_smallest_thing_first() {
    let mut app = test_app();
    app.page = Page::Dashboard;
    app.history.push(Page::Home);

    // Into the input, out of it — and what was typed survives, because
    // walking away from a search is not undoing it.
    app.handle_key(key(KeyCode::Char('/')));
    for character in ['w', 'i', 'p'] {
        app.handle_key(key(KeyCode::Char(character)));
    }
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.focus, Focus::List);
    assert_eq!(app.query, "wip", "Esc left the input, not the search");

    // The next one drops the search and reloads.
    assert_eq!(app.handle_key(key(KeyCode::Esc)), Action::Refresh);
    assert!(app.query.is_empty());
    assert_eq!(app.page, Page::Dashboard, "and it stays on the page");

    // Only then does Esc leave.
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.page, Page::Home);
}

#[test]
fn the_help_page_is_only_shortcuts() {
    // The drawing used to sit in the free space here and was removed: help
    // is a page somebody opens to find a key, and art beside the list is
    // one more thing to read past.
    let mut app = test_app();
    app.page = Page::Help;
    let mut terminal = Terminal::new(TestBackend::new(120, 44)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    assert!(drawn.contains("copy the selected memory to the clipboard"));
    assert!(
        !drawn
            .chars()
            .any(|c| ('\u{2801}'..='\u{28FF}').contains(&c)),
        "the help page should carry no drawing:\n{drawn}"
    );
}

#[test]
fn tab_shows_each_list_in_turn() {
    // One list at a time, and Tab is what swaps it. The filter panel is not
    // one of its stops: Tab means "show me the next list".
    let mut app = test_app();
    app.page = Page::Dashboard;
    assert_eq!(app.list, ListKind::Observations, "observations to start");

    for expected in [
        ListKind::Sessions,
        ListKind::Prompts,
        ListKind::Observations,
    ] {
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.list, expected);
    }
}

#[test]
fn the_arrow_keys_move_only_what_is_showing() {
    // The failure this guards is a cursor that looks alive and is not: the
    // sessions panel once drew one that no key could reach.
    let mut app = test_app();
    app.page = Page::Dashboard;
    app.stats.projects = vec!["leteo".to_owned(), "engram".to_owned()];
    app.recent = (1..=3).map(|n| observation(n, "memory")).collect();
    app.prompts = std::iter::repeat_n(app.prompts[0].clone(), 3).collect();
    app.sessions = std::iter::repeat_n(app.sessions[0].clone(), 3).collect();

    let cursors = |app: &App| {
        (
            app.recent_selected,
            app.session_selected,
            app.prompt_selected,
            app.project_selected,
        )
    };
    let cases = [
        (Some(ListKind::Observations), "recent"),
        (Some(ListKind::Sessions), "session"),
        (Some(ListKind::Prompts), "prompt"),
        (None, "project"),
    ];
    for (list, moved) in cases {
        match list {
            Some(list) => show_list(&mut app, list),
            // None means the filter panel, which is reached with f.
            None => {
                app.handle_key(key(KeyCode::Char('f')));
            }
        }
        let before = cursors(&app);
        app.handle_key(key(KeyCode::Char('j')));
        let after = cursors(&app);
        for (name, did) in [
            ("recent", before.0 != after.0),
            ("session", before.1 != after.1),
            ("prompt", before.2 != after.2),
            ("project", before.3 != after.3),
        ] {
            assert_eq!(
                did,
                name == moved,
                "showing {list:?}, {name} should{} have moved",
                if name == moved { "" } else { " not" }
            );
        }
    }
}

#[test]
fn the_home_screen_lists_the_menu_and_moves_through_it() {
    let mut app = test_app();
    app.page = Page::Home;

    let mut terminal = Terminal::new(TestBackend::new(100, 50)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    for (label, _) in MENU {
        let label = label(crate::i18n::screens(crate::settings::Interface::English));
        assert!(drawn.contains(label), "menu is missing {label}:\n{drawn}");
    }
    assert!(
        drawn.contains(crate::sardi::CAT_LARGE[14]),
        "and the drawing, in a window this size:\n{drawn}"
    );
    assert!(
        drawn.contains(&format!(
            "\u{25b8} {}",
            (MENU[0].0)(crate::i18n::screens(crate::settings::Interface::English))
        )),
        "the cursor should start on the first entry:\n{drawn}"
    );

    app.move_selection(1);
    assert_eq!(app.home_selected, 1);
    // It stops at the ends rather than wrapping: a cursor that jumps from
    // the last entry back to the first invites a mis-press.
    app.move_selection(-1);
    app.move_selection(-1);
    assert_eq!(app.home_selected, 0);
    for _ in 0..MENU.len() * 2 {
        app.move_selection(1);
    }
    assert_eq!(app.home_selected, MENU.len() - 1);
}

#[test]
fn every_menu_entry_goes_where_it_says() {
    // The labels and the targets are one table, and this is what keeps the
    // table honest: selecting an entry has to land where it names.
    for (index, (label, target)) in MENU.iter().enumerate() {
        let label = label(crate::i18n::screens(crate::settings::Interface::English));
        let mut app = test_app();
        app.page = Page::Home;
        app.home_selected = index;
        let action = app.activate_selection();
        match target {
            MenuTarget::Quit => {
                assert_eq!(action, Action::Quit, "{label} should quit");
                assert_eq!(app.page, Page::Home, "{label} should not navigate");
            }
            // Asks before it does anything, like every other removal here.
            // Landing straight on `Action::Uninstall` would take the machine
            // apart on one keypress from the menu.
            MenuTarget::Uninstall => {
                assert_eq!(
                    action,
                    Action::ConfirmUninstall,
                    "{label} has to ask before it removes anything"
                );
                assert_eq!(app.page, Page::Home, "{label} should not navigate");
            }
            MenuTarget::Open(page) => {
                assert_eq!(action, Action::None, "{label} should not raise an action");
                assert_eq!(app.page, *page, "{label} should open {page:?}");
                // And Esc has to come back, or the menu is a one-way door.
                app.back();
                assert_eq!(app.page, Page::Home, "Esc should return home from {label}");
            }
        }
    }
}

#[test]
fn the_setup_page_is_the_same_flow_as_leteo_setup() {
    // The point of the shared wizard: what the setup page shows has to be
    // what `leteo setup` shows. Two renderers, one set of questions — and
    // if this ever diverges, one of them has grown its own copy.
    let mut app = test_app();
    app.navigate(Page::Setup);
    let wizard = app.wizard.as_ref().expect("setup starts the wizard");
    let expected: Vec<String> = wizard.render().into_iter().map(|row| row.text).collect();

    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    for line in expected.iter().filter(|line| !line.trim().is_empty()) {
        assert!(
            drawn.contains(line.trim()),
            "the page is missing a line the wizard rendered: {line:?}\n{drawn}"
        );
    }
}

#[test]
fn the_options_page_opens_on_the_list_of_settings_and_takes_the_wizard_keys() {
    // The entry the home menu grew. These are answers for the whole store, and
    // reaching them used to mean walking the agent setup — every question along
    // the way a chance to change something nobody came to change.
    let mut app = test_app();
    app.navigate(Page::Options);

    let wizard = app.wizard.as_ref().expect("the options page starts one");
    assert_eq!(
        wizard.step(),
        crate::setup::wizard::Step::Options,
        "it should open on the list, not on a question and not on the agents"
    );

    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    let say = crate::i18n::screens(crate::settings::Interface::English);
    assert!(
        drawn.contains(say.options_question.trim()),
        "the list should be on screen:\n{drawn}"
    );
    for setting in [
        say.option_interface.to_owned(),
        say.option_memory_language.to_owned(),
        crate::i18n::fill(say.option_voice, "name", crate::sardi::NAME),
    ] {
        assert!(
            drawn.contains(setting.trim()),
            "every setting should be listed, and {setting} is not:\n{drawn}"
        );
    }
    assert!(
        drawn.contains(say.panel_options.trim()),
        "and the border should say which page this is:\n{drawn}"
    );
    assert!(
        !drawn.contains(say.choose_agents.trim()),
        "the agent question belongs to the other page:\n{drawn}"
    );
    // The footer offers the keys this page takes, which are not the setup ones.
    assert!(
        drawn.contains(say.keys_options.trim()),
        "the footer should say what the keys do here:\n{drawn}"
    );

    // The wizard owns the keys here as it does on the setup page: `s` must not
    // jump to the sessions list from inside a flow.
    app.handle_key(key(KeyCode::Char('s')));
    assert_eq!(app.page, Page::Options);
}

#[test]
fn choosing_a_language_repaints_the_window_around_the_flow() {
    // The wizard repaints as the answer is picked — the question is asked in
    // the language being chosen, so it has to. Everything around it read a
    // language once, when the dashboard opened, so the header, the footer and
    // the panel borders stayed in the old one until the next start. A setting
    // that visibly does not take is one somebody sets twice.
    // Its own directory rather than the shared fixture's: this test changes a
    // settings file, the tests run in one process at the same time, and a
    // fixture that answers "Spanish" for a moment is a fixture the rest of them
    // can catch mid-change.
    let temp = tempfile::TempDir::new().unwrap();
    crate::settings::save(
        temp.path(),
        &crate::settings::Settings {
            interface: Some(crate::settings::Interface::English),
            ..crate::settings::Settings::default()
        },
    )
    .unwrap();
    let mut app = test_app();
    app.database_path = temp.path().join("leteo.db").display().to_string();
    assert_eq!(app.interface, crate::settings::Interface::English);
    app.navigate(Page::Options);

    app.handle_key(key(KeyCode::Enter)); // open the interface language
    app.handle_key(key(KeyCode::Down)); // español
    app.handle_key(key(KeyCode::Char(' ')));

    // Before leaving, and this is the half that used to be missing: the window
    // around the page has to turn with it, not at the next start.
    assert_eq!(
        app.interface,
        crate::settings::Interface::Spanish,
        "the window kept the old language while the page showed the new one"
    );

    app.handle_key(key(KeyCode::Enter)); // back to the list
    app.handle_key(key(KeyCode::Esc)); // and out, which is what saves

    assert_eq!(
        app.page,
        Page::Dashboard,
        "leaving the list goes back where it was opened from"
    );
    assert_eq!(
        crate::settings::load(temp.path()).interface,
        Some(crate::settings::Interface::Spanish),
        "the answer never reached the file"
    );
    assert_eq!(
        app.interface,
        crate::settings::Interface::Spanish,
        "the window kept the language the page had just changed"
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    let spanish = crate::i18n::screens(crate::settings::Interface::Spanish);
    assert!(
        drawn.contains(spanish.page_dashboard),
        "the header should be in the new language:\n{drawn}"
    );
}

#[test]
fn the_setup_page_takes_the_wizard_keys_and_not_the_page_shortcuts() {
    let mut app = test_app();
    app.navigate(Page::Setup);

    // `s` would jump to Sessions anywhere else. Inside a flow it must not.
    app.handle_key(key(KeyCode::Char('s')));
    assert_eq!(app.page, Page::Setup, "a letter should not leave the flow");

    // Space toggles the agent under the cursor, which the shared handler
    // has no notion of at all.
    //
    // Counted rather than looked for, because the wizard opens ticked
    // wherever Leteo is already installed. The probe points at an empty
    // sandbox above, so nothing starts ticked here — but counting says what
    // the key does rather than what the machine happens to hold.
    let ticks = |app: &App| {
        app.wizard
            .as_ref()
            .expect("still open")
            .render()
            .into_iter()
            .filter(|row| row.text.contains('\u{2713}'))
            .count()
    };
    let before = ticks(&app);
    app.handle_key(key(KeyCode::Char(' ')));
    assert_ne!(
        ticks(&app),
        before,
        "space should toggle the agent under the cursor"
    );

    // Esc cancels the flow and leaves the page, rather than only stepping
    // back one question.
    app.handle_key(key(KeyCode::Esc));
    assert_ne!(app.page, Page::Setup);
    assert!(app.wizard.is_none(), "leaving setup drops the wizard");
    let status = app.status.clone().expect("cancelling says so");
    assert!(status.text.contains("cancelled"), "{}", status.text);
    assert!(!status.is_error, "cancelling is not a failure");
}

#[test]
fn the_home_screen_drops_the_drawing_before_the_menu() {
    // The menu is the point of the screen. In a window too small for both,
    // the drawing is what gives way.
    let mut app = test_app();
    app.page = Page::Home;
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    assert!(
        !drawn.contains(crate::sardi::CAT_LARGE[14]),
        "the drawing does not fit here:\n{drawn}"
    );
    for (label, _) in MENU {
        let label = label(crate::i18n::screens(crate::settings::Interface::English));
        assert!(
            drawn.contains(label),
            "but every entry must survive, missing {label}:\n{drawn}"
        );
    }
}

#[test]
fn the_ends_of_a_list_are_one_key_away() {
    // Three thousand rows is an hour of `j`. End and Home are what make the
    // far end of a store reachable at all.
    let height = 28;
    let (mut selected, mut offset) = (0, 0);
    let far = |selected: &mut usize, offset: &mut usize, delta| {
        App::step(selected, offset, WINDOW, 3313, height, delta)
    };
    assert!(far(&mut selected, &mut offset, isize::MAX));
    assert_eq!(offset + selected, 3312, "the last row, not past it");

    assert!(far(&mut selected, &mut offset, isize::MIN));
    assert_eq!((offset, selected), (0, 0), "and back to the first");

    // Already there is not a reload for the sake of one.
    assert!(!far(&mut selected, &mut offset, isize::MIN));
    assert!(!far(&mut selected, &mut offset, -1));
}

#[test]
fn the_page_keys_move_by_what_was_last_on_screen() {
    // PgDn means "a screenful", and only the frame that was drawn knows how
    // much that is. A number chosen in the source would move by something
    // other than what somebody just looked at.
    let (_temp, mut store) = store_of(60);
    let mut app = App::load(&mut store).unwrap();
    app.page = Page::Dashboard;

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let height = app.list_height.get();
    assert!(height > 0 && height < 60, "a partial view of the list");

    app.handle_key(key(KeyCode::PageDown));
    assert_eq!(app.recent_selected, height, "one screenful down");
    app.handle_key(key(KeyCode::PageUp));
    assert_eq!(app.recent_selected, 0);

    app.handle_key(key(KeyCode::End));
    assert_eq!(app.recent_selected, 59, "the last row of the list");
    app.handle_key(key(KeyCode::Home));
    assert_eq!(app.recent_selected, 0);
}

#[test]
fn the_cloud_page_reports_replication_without_secrets() {
    let mut app = test_app();
    app.page = Page::Cloud;
    let backend = TestBackend::new(90, 16);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| render(frame, &app)).unwrap();

    let rendered = buffer_text(terminal.backend());
    assert!(rendered.contains("https://memory.example.com"));
    assert!(rendered.contains("enabled"));
    assert!(rendered.contains("4 mutation(s)"));
    assert!(!rendered.to_lowercase().contains("token"));
}
