//! What actually reaches the screen.

use super::*;
use crate::memory::model::CaveatVerb;

#[test]
fn the_dashboard_renders_with_test_backend() {
    let app = test_app();
    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| render(frame, &app)).unwrap();
    let dashboard = buffer_text(terminal.backend());
    assert!(dashboard.contains("LETEO"));
    assert!(dashboard.contains("OBSERVATIONS"));
    assert!(dashboard.contains("First memory"));
    // One list at a time now, so the others are a Tab away rather than
    // sharing the screen with it.
    assert!(!dashboard.contains(" Sessions "));
    // The query row is there before anybody has typed anything, saying what
    // the key is for. Hidden until used, the only thing on screen that said
    // searching was possible was a word in the footer.
    assert!(has_query_row(&dashboard), "{dashboard}");
    assert!(
        dashboard.contains("/ search memories"),
        "and it has to say what it is:\n{dashboard}"
    );
}

#[test]
fn sardi_keeps_its_own_language_on_a_screen_that_is_in_another() {
    // The two settings are separate, so the screen where they differ has to be
    // legible rather than look like a rendering fault: the panels, the counters
    // and the page name are Leteo's, and the line beside them is Sardi's.
    let mut app = test_app();
    app.interface = crate::settings::Interface::Spanish;
    app.voice_interface = crate::settings::Interface::English;
    let mut terminal = Terminal::new(TestBackend::new(110, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());

    let spanish = crate::i18n::screens(crate::settings::Interface::Spanish);
    assert!(
        drawn.contains(spanish.stat_observations),
        "the screens follow their own setting:\n{drawn}"
    );
    // Without the mark it opens with: that is one wide glyph over two cells,
    // and reading the buffer back turns it into two characters.
    let said = |language| {
        let line = crate::sardi::watching(
            language,
            app.stats.total_observations,
            app.stats.projects.len(),
        );
        line.split_once(' ')
            .expect("a mark, then the sentence")
            .1
            .to_owned()
    };
    assert!(
        drawn.contains(&said(crate::settings::Interface::English)),
        "and the voice follows its own:\n{drawn}"
    );
    assert!(
        !drawn.contains(&said(crate::settings::Interface::Spanish)),
        "the voice was painted in the screens' language:\n{drawn}"
    );
}

#[test]
fn an_empty_dashboard_says_what_happens_next() {
    // Four zeros and three empty lists tell somebody their setup is broken
    // when it is only new. The drawing lives on the home screen, so what is
    // left here is the sentence — which is the part that helps.
    let mut app = test_app();
    app.stats = Stats {
        total_sessions: 0,
        total_observations: 0,
        total_prompts: 0,
        projects: Vec::new(),
    };
    app.recent.clear();
    app.sessions.clear();

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    assert!(
        drawn.contains("nothing to look after yet"),
        "the empty dashboard has to say what happens next:\n{drawn}"
    );
    assert!(
        !drawn.contains("OBSERVATIONS"),
        "the panels of zeros are what it replaces:\n{drawn}"
    );
    assert!(
        !drawn
            .chars()
            .any(|c| ('\u{2801}'..='\u{28FF}').contains(&c)),
        "the same cat one keypress from home reads as a placeholder:\n{drawn}"
    );
}

#[test]
fn the_wordmark_stands_in_for_the_drawing_rather_than_joining_it() {
    // The cat is the mark. Printing both says the same thing twice, and the
    // three rows the box costs are the rows that decide whether the cat fits.
    let mut app = test_app();
    app.page = Page::Home;

    // Room for the pair: the drawing carries it, and the box stays away.
    let mut roomy = Terminal::new(TestBackend::new(100, 40)).unwrap();
    roomy.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(roomy.backend());
    assert!(drawn.contains(crate::sardi::CAT_LARGE[14]), "{drawn}");
    assert!(
        !has_wordmark(&drawn),
        "the wordmark should stay away while the cat is on screen:\n{drawn}"
    );

    // No room: the box is what identifies the screen instead.
    let mut cramped = Terminal::new(TestBackend::new(60, 20)).unwrap();
    cramped.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(cramped.backend());
    assert!(!drawn.contains(crate::sardi::CAT_LARGE[14]), "{drawn}");
    assert!(
        has_wordmark(&drawn),
        "without the cat the wordmark has to appear:\n{drawn}"
    );
}

#[test]
fn the_trimmings_go_before_the_wordmark_does() {
    // Graded by worth, not by size. Beside the drawing there is room for a
    // heading and a version; stacked, those rows belong to the wordmark,
    // which is the only thing naming the screen once the cat is gone.
    let mut app = test_app();
    app.page = Page::Home;

    let mut roomy = Terminal::new(TestBackend::new(100, 40)).unwrap();
    roomy.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(roomy.backend());
    assert!(drawn.contains("ACTIONS"), "{drawn}");
    assert!(
        drawn.contains(&format!("leteo {}", env!("CARGO_PKG_VERSION"))),
        "the version belongs beside the drawing:\n{drawn}"
    );

    let mut cramped = Terminal::new(TestBackend::new(60, 20)).unwrap();
    cramped.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(cramped.backend());
    assert!(has_wordmark(&drawn), "{drawn}");
    assert!(
        !drawn.contains("ACTIONS"),
        "the heading should give up its row here:\n{drawn}"
    );
    for (label, _) in MENU {
        let label = label(crate::i18n::screens(crate::settings::Interface::English));
        assert!(drawn.contains(label), "but not an entry: {label}\n{drawn}");
    }
}

#[test]
fn the_drawing_is_banded_top_to_bottom() {
    // Verified through the buffer rather than the terminal on purpose: on
    // Windows crossterm sets colour through the console API instead of
    // escape sequences, so reading the byte stream shows no colour at all
    // even when it is working. The buffer records what was asked for.
    let mut app = test_app();
    app.page = Page::Home;
    let mut terminal = Terminal::new(TestBackend::new(120, 50)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();

    // Collect the colour of every cell holding a lit Braille pattern.
    let mut by_row: Vec<(u16, Color)> = Vec::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            let symbol = cell.symbol();
            if symbol
                .chars()
                .next()
                .is_some_and(|c| ('\u{2801}'..='\u{28FF}').contains(&c))
            {
                by_row.push((y, cell.fg));
                break;
            }
        }
    }

    assert!(
        by_row.len() >= 4,
        "expected several rows of Braille, found {}",
        by_row.len()
    );
    let first = by_row.first().unwrap().1;
    let last = by_row.last().unwrap().1;
    assert_ne!(
        first, last,
        "the top and bottom of the drawing should not be the same colour"
    );
    let distinct: std::collections::BTreeSet<_> = by_row
        .iter()
        .map(|(_, colour)| format!("{colour:?}"))
        .collect();
    assert!(
        distinct.len() >= 3,
        "banding should give several colours, saw {distinct:?}"
    );
    // And every one has to be a real colour, not the terminal default.
    for (row, colour) in &by_row {
        assert!(
            matches!(colour, Color::Rgb(_, _, _)),
            "row {row} was left at {colour:?}"
        );
    }
}

#[test]
fn a_dashboard_with_memories_is_unaffected() {
    // The empty state must not trigger on a store that simply has no
    // sessions yet, or someone's memories would vanish behind a cat.
    let mut app = test_app();
    app.sessions.clear();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    assert!(drawn.contains("OBSERVATIONS"), "{drawn}");
    assert!(drawn.contains("First memory"), "{drawn}");
    assert!(!drawn.contains(crate::sardi::CAT_LARGE[5]), "{drawn}");
}

#[test]
fn every_view_renders_in_a_small_terminal_without_panicking() {
    let mut app = test_app();
    app.detail = Some(app.recent[0].clone());
    app.timeline = Some(timeline(app.recent[0].clone()));
    let backend = TestBackend::new(28, 9);
    let mut terminal = Terminal::new(backend).unwrap();

    for page in [
        Page::Dashboard,
        Page::Detail,
        Page::Session,
        Page::Timeline,
        Page::Setup,
        Page::Options,
        Page::Cloud,
        Page::Help,
    ] {
        // Through `navigate` rather than by assigning the field, which is what
        // the two pages behind the wizard need: assigned directly they arrive
        // with no wizard, the painter returns early, and the panel this claims
        // to have rendered was an empty box. The screens with the most rows to
        // fit in nine were the two it was not drawing.
        app.navigate(page);
        terminal.draw(|frame| render(frame, &app)).unwrap();
        assert!(
            buffer_text(terminal.backend())
                .contains(page.title(crate::settings::Interface::English))
        );
    }

    // And every screen of the flow, at every size from "no room at all" up to
    // one that fits: the window is arithmetic over three slices, and the height
    // it is handed comes from whatever the terminal happens to be.
    let mut wizard = crate::setup::wizard::Wizard::preferences(crate::setup::wizard::offer(
        std::path::Path::new(&app.database_path),
        true,
        &app.setup_probe,
    ));
    let fits_every_window = |wizard: &crate::setup::wizard::Wizard| {
        let whole = wizard.render();
        for height in 0..30 {
            let shown = wizard.render_within(height);
            if height >= whole.len() {
                assert_eq!(shown, whole, "a window with room to spare narrowed it");
                continue;
            }
            assert!(
                shown.len() <= height,
                "{height} rows asked for, {} painted on {:?}",
                shown.len(),
                wizard.step()
            );
            // A window that fits by dropping the row the cursor is on is a
            // window somebody cannot steer, which is the fault this replaced.
            // True down to a single row: what one row of a question is worth
            // is the answer being pointed at.
            let has_cursor = |rows: &[crate::setup::wizard::Row]| {
                rows.iter()
                    .any(|row| row.role == crate::setup::wizard::Role::Focused)
            };
            assert!(
                height == 0 || !has_cursor(&whole) || has_cursor(&shown),
                "the cursor fell out of a {height}-row window on {:?}",
                wizard.step()
            );
        }
    };

    // Every screen of the flow: the index, and each of the screens behind its
    // rows — the tall ones, where the window has to do something.
    for _ in 0..6 {
        fits_every_window(&wizard);
        wizard.advance();
        fits_every_window(&wizard);
        wizard.back();
        wizard.down();
    }
}

#[test]
fn a_memory_the_graph_has_overturned_says_so_on_screen() {
    // The agent is told on a prompt, at a session opening, and in the reply to
    // `mem_get_observation`. A person reading the same memory here was the one
    // surface left showing a stale decision as though it still held.
    let mut app = test_app();
    app.open_detail(
        app.recent[0].clone(),
        vec![Caveat {
            verb: CaveatVerb::SupersededBy,
            other_id: 1081,
            other_title: "Restructured navigation".to_owned(),
        }],
    );
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();

    terminal.draw(|frame| render(frame, &app)).unwrap();
    let screen = buffer_text(terminal.backend());

    assert!(screen.contains("superseded by"), "{screen}");
    assert!(screen.contains("#1081"), "{screen}");
    assert!(
        screen.contains("Restructured navigation"),
        "naming the count without naming what replaced it is not actionable:\n{screen}"
    );
    // The facts pane grew rather than pushing the content off: both are on
    // screen together.
    assert!(screen.contains("Content"), "{screen}");
}

#[test]
fn a_memory_that_still_holds_says_nothing_extra() {
    let mut app = test_app();
    app.open_detail(app.recent[0].clone(), Vec::new());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();

    terminal.draw(|frame| render(frame, &app)).unwrap();
    let screen = buffer_text(terminal.backend());

    assert!(!screen.contains("superseded"), "{screen}");
    assert!(!screen.contains("conflicts with"), "{screen}");
}

/// No page is painted with a template placeholder still in it.
///
/// The same hole the wizard had, in the half that renders through ratatui. A
/// call site that forgets to fill `{count}` or `{query}` compiles, and the
/// screen shows the placeholder to somebody's face. The dashboard alone fills
/// nine of these and the delete confirmations twenty-nine.
///
/// Both languages, because a template is per-language and only one of them has
/// to be wired wrongly for this to matter.
#[test]
fn no_page_is_painted_with_a_placeholder_still_in_it() {
    for language in crate::settings::Interface::ALL {
        let mut app = test_app();
        app.interface = language;
        app.detail = Some(app.recent[0].clone());
        app.timeline = Some(timeline(app.recent[0].clone()));
        // A search running and a project filter on, so the titles that carry
        // `{query}` and `{project}` are the ones drawn.
        app.query = "sqlite".to_owned();
        app.projects_filter = vec!["leteo".to_owned()];
        // Wide enough that a placeholder is not simply cut off the edge, which
        // would hide the very thing this looks for.
        let mut terminal = Terminal::new(TestBackend::new(200, 60)).unwrap();

        for page in [
            Page::Home,
            Page::Dashboard,
            Page::Detail,
            Page::Session,
            Page::Timeline,
            Page::Cloud,
            Page::Help,
        ] {
            app.page = page;
            for list in [
                ListKind::Observations,
                ListKind::Sessions,
                ListKind::Prompts,
            ] {
                app.list = list;
                terminal.draw(|frame| render(frame, &app)).unwrap();
                let drawn = buffer_text(terminal.backend());
                assert!(
                    crate::i18n::unfilled_placeholder(&drawn).is_none(),
                    "{language:?} left {:?} unfilled on {page:?}/{list:?}:\n{drawn}",
                    crate::i18n::unfilled_placeholder(&drawn).unwrap()
                );
            }
        }
    }
}
