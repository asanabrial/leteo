//! Fixtures every area shares, and the tests that belong to no one area.

use super::*;
use crate::memory::model::{Session, TimelineEntry};
use ratatui::backend::TestBackend;

mod actions;
mod lists;
mod navigation;
mod rendering;
mod search;

/// Presses Tab until the wanted list is the one showing.
///
/// Bounded so a broken cycle fails the test rather than hanging it.
fn show_list(app: &mut App, wanted: ListKind) {
    for _ in 0..6 {
        if app.list == wanted && app.focus == Focus::List {
            return;
        }
        app.handle_key(key(KeyCode::Tab));
    }
    panic!("Tab never reached {wanted:?}");
}

/// Where the wordmark box would be, if it were drawn. Looked for by its
/// frame rather than by the word: the header badge also says LETEO, and
/// asserting on the word alone passes whether the box is there or not.
fn has_wordmark(drawn: &str) -> bool {
    drawn.contains('\u{2554}') && drawn.contains("LETEO")
}

/// A store with `count` observations in one project, ready to page through.
fn store_of(count: usize) -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("tui.db"))).unwrap();
    // `App::load` resolves the interface language, which follows the machine
    // when nothing is set — so without this the screens below are drawn in
    // whatever language the developer's computer is in, and every assertion
    // about an English label is a test of that instead of of the dashboard.
    crate::settings::save(
        temp.path(),
        &crate::settings::Settings {
            interface: Some(crate::settings::Interface::English),
            ..crate::settings::Settings::default()
        },
    )
    .unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for n in 0..count {
        store
            .add_observation(crate::memory::model::AddObservation {
                session_id: "s1".to_owned(),
                kind: "decision".to_owned(),
                title: format!("Memory {n:03}"),
                content: "body".to_owned(),
                tool_name: None,
                project: Some("leteo".to_owned()),
                scope: "project".to_owned(),
                topic_key: None,
                prompt_sync_id: None,
            })
            .unwrap();
    }
    (temp, store)
}

#[test]
fn enter_opens_a_session_and_what_it_recorded() {
    // The dashboard listed sessions with a count on each and no way past
    // them, which made the count a fact nobody could act on.
    let temp = tempfile::tempdir().unwrap();
    let mut store =
        Store::open(crate::store::StoreConfig::new(temp.path().join("tui.db"))).unwrap();
    crate::settings::save(
        temp.path(),
        &crate::settings::Settings {
            interface: Some(crate::settings::Interface::English),
            ..crate::settings::Settings::default()
        },
    )
    .unwrap();
    store.create_session("s1", "leteo", "C:/repo").unwrap();
    for title in ["First thing", "Second thing"] {
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
    app.list = ListKind::Sessions;

    let action = app.activate_selection();
    assert_eq!(action, Action::OpenSession("s1".to_owned()));
    assert!(!apply_action(&mut app, &mut store, action));
    assert_eq!(app.page, Page::Session);

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let drawn = buffer_text(terminal.backend());
    assert!(drawn.contains("First thing"), "{drawn}");
    assert!(drawn.contains("Second thing"), "{drawn}");
    assert!(drawn.contains("Recorded (2)"), "{drawn}");

    // And the rows are a way in, not a wall of text: Enter opens one.
    app.handle_key(key(KeyCode::Char('j')));
    let Action::OpenObservation(id) = app.activate_selection() else {
        panic!("Enter on a recorded memory should open it");
    };
    assert_eq!(
        store.get_observation(id).unwrap().title,
        "Second thing",
        "and it opens the one under the cursor"
    );

    // Esc comes back to where it was opened from.
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.page, Page::Dashboard);
}

#[test]
fn deleting_a_session_takes_everything_it_recorded() {
    let (_temp, mut store) = store_of(4);
    store
        .add_prompt(crate::memory::model::AddPrompt {
            session_id: "s1".to_owned(),
            content: "why?".to_owned(),
            project: Some("leteo".to_owned()),
        })
        .unwrap();
    let mut app = App::load(&mut store).unwrap();
    app.page = Page::Dashboard;
    app.list = ListKind::Sessions;

    let action = app.handle_key(key(KeyCode::Char('D')));
    assert_eq!(
        action,
        Action::Confirm {
            what: Target::Session("s1".to_owned()),
            hard: true
        }
    );
    apply_action(&mut app, &mut store, action);
    let pending = app.pending.clone().expect("a confirmation is up");
    // The counts come from the store rather than from the row, which under
    // a search carries the number of matches instead.
    assert!(pending.heading.contains("Delete session s1"));
    assert!(
        pending.detail.iter().any(|line| line == "4 memory(s)"),
        "{:?}",
        pending.detail
    );
    assert!(pending.detail.iter().any(|line| line == "1 prompt(s)"));
    assert!(
        pending
            .detail
            .last()
            .is_some_and(|line| line.contains("cannot be undone"))
    );

    let confirmed = app.handle_key(key(KeyCode::Char('y')));
    apply_action(&mut app, &mut store, confirmed);
    assert!(app.sessions.is_empty(), "the session went");
    assert!(app.recent.is_empty(), "and so did what it recorded");
    assert!(app.prompts.is_empty());
    assert_eq!(store.session_counts("s1").unwrap(), (0, 0));
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Whether the dashboard's query row is on screen.
///
/// By its shape rather than by its text: `/ ` also appears in the footer's
/// list of keys, and matching that instead would make this pass whatever the
/// dashboard did.
fn has_query_row(drawn: &str) -> bool {
    drawn
        .lines()
        .any(|line| line.trim_start().starts_with("/ "))
}

/// A directory the fixture's database can claim to live in, with the interface
/// language pinned in it.
///
/// The path used to be a bare `C:\leteo.db`, which reads as harmless and is not:
/// anything the dashboard opens that re-reads the settings — the wizard does, on
/// every `navigate` — looks beside the database, finds no file, and falls back
/// to the machine's own language. So the chrome painted the English these tests
/// assert while the wizard inside it painted Spanish, on a Spanish machine and
/// nowhere else.
fn fixture_data_dir() -> &'static std::path::Path {
    static DIRECTORY: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            let directory = tempfile::TempDir::new().expect("a temporary directory");
            crate::settings::save(
                directory.path(),
                &crate::settings::Settings {
                    interface: Some(crate::settings::Interface::English),
                    ..crate::settings::Settings::default()
                },
            )
            .expect("pin the fixture's language");
            directory
        })
        .path()
}

fn test_app() -> App {
    App {
        page: Page::Dashboard,
        history: Vec::new(),
        database_path: fixture_data_dir().join("leteo.db").display().to_string(),
        // Pinned rather than detected: the assertions below read English
        // labels, and the real `load` falls back to the machine's language.
        // The same answer sits in the settings file beside the database above,
        // so anything that re-reads it agrees with this rather than with the
        // machine.
        interface: crate::settings::Interface::English,
        // Pinned alongside it rather than left to follow: a test that means to
        // exercise the two languages differing says so by setting this.
        voice_interface: crate::settings::Interface::English,
        stats: Stats {
            total_sessions: 1,
            total_observations: 1,
            total_prompts: 3,
            projects: vec!["leteo".to_owned()],
        },
        recent: vec![observation(1, "First memory")],
        sessions: vec![SessionSummary {
            id: "session-1".to_owned(),
            project: "leteo".to_owned(),
            started_at: "2026-07-27 10:00:00".to_owned(),
            last_activity: "2026-07-27 10:00:00".to_owned(),
            ended_at: None,
            summary: Some("TUI work".to_owned()),
            observation_count: 1,
        }],
        prompts: vec![Prompt {
            id: 1,
            sync_id: "prm-1".to_owned(),
            session_id: "session-1".to_owned(),
            content: "why does the wizard look plain?".to_owned(),
            project: "leteo".to_owned(),
            created_at: "2026-07-27 10:00:00".to_owned(),
        }],
        detail: None,
        detail_caveats: Vec::new(),
        session: None,
        timeline: None,
        query: String::new(),
        recent_selected: 0,
        session_selected: 0,
        session_entry_selected: 0,
        recent_offset: 0,
        session_offset: 0,
        prompt_offset: 0,
        session_entry_offset: 0,
        recent_total: 1,
        session_total: 1,
        prompt_total: 3,
        session_entry_total: 0,
        timeline_selected: 0,
        home_selected: 0,
        project_selected: 0,
        prompt_selected: 0,
        // An empty sandbox, so no test's result depends on which agents
        // the machine running it happens to have installed.
        setup_probe: crate::setup::SetupOptions {
            home_dir: Some(std::env::temp_dir().join("leteo-tui-tests-none")),
            config_home: Some(std::env::temp_dir().join("leteo-tui-tests-none")),
            app_data: Some(std::env::temp_dir().join("leteo-tui-tests-none")),
            ..crate::setup::SetupOptions::default()
        },
        list_height: Cell::new(ASSUMED_HEIGHT),
        list: ListKind::Observations,
        focus: Focus::List,
        projects_filter: Vec::new(),
        wizard: None,
        detail_scroll: 0,
        status: None,
        pending: None,
        cloud: CloudOverview {
            configured: true,
            server: "https://memory.example.com".to_owned(),
            enabled: true,
            projects: vec!["leteo".to_owned()],
            enrolled: vec!["leteo".to_owned()],
            pending_mutations: 4,
            deferred: 0,
            dead: 0,
            unreadable: None,
            state: None,
        },
        exit: Exit::Normal,
    }
}

fn observation(id: i64, title: &str) -> Observation {
    Observation {
        id,
        sync_id: format!("obs-{id}"),
        session_id: "session-1".to_owned(),
        kind: "learning".to_owned(),
        title: title.to_owned(),
        content: "A detailed observation body".to_owned(),
        tool_name: None,
        project: Some("leteo".to_owned()),
        scope: "project".to_owned(),
        topic_key: Some("ui/tui".to_owned()),
        revision_count: 0,
        duplicate_count: 0,
        last_seen_at: None,
        review_after: None,
        prompt_sync_id: None,
        pinned: false,
        created_at: "2026-07-27 10:00:00".to_owned(),
        updated_at: "2026-07-27 10:00:00".to_owned(),
        deleted_at: None,
    }
}

fn timeline(focus: Observation) -> TimelineResult {
    let entry = TimelineEntry {
        id: focus.id + 1,
        session_id: focus.session_id.clone(),
        kind: "discovery".to_owned(),
        title: "Following memory".to_owned(),
        content: "After the focus".to_owned(),
        tool_name: None,
        project: focus.project.clone(),
        scope: focus.scope.clone(),
        topic_key: None,
        revision_count: 0,
        duplicate_count: 0,
        last_seen_at: None,
        created_at: "2026-07-27 10:01:00".to_owned(),
        updated_at: "2026-07-27 10:01:00".to_owned(),
        deleted_at: None,
        is_focus: false,
    };
    TimelineResult {
        focus,
        before: Vec::new(),
        after: vec![entry],
        session_info: Some(Session {
            id: "session-1".to_owned(),
            project: "leteo".to_owned(),
            directory: "C:\\repo".to_owned(),
            started_at: "2026-07-27 10:00:00".to_owned(),
            ended_at: None,
            summary: None,
        }),
        before_total: 1,
        after_total: 0,
    }
}

fn buffer_text(backend: &TestBackend) -> String {
    let buffer = backend.buffer();
    let mut output = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            output.push_str(buffer.cell((x, y)).unwrap().symbol());
        }
        output.push('\n');
    }
    output
}

/// A queue nobody could read must not look like a queue with nothing in it.
///
/// The page reported "none enrolled, 0 queued, 0 deferred, 0 dead" whether the
/// store answered or failed, because each count defaulted on error. Those are
/// the same four words a healthy idle store produces — and this is the page
/// somebody opens *because* they suspect replication is stuck, so the one
/// reading it must never give is a clean bill of health it did not earn.
#[test]
fn a_replication_queue_that_cannot_be_read_says_so_rather_than_showing_zero() {
    let (_temp, store) = store_of(1);
    let mut app = test_app();
    app.page = Page::Cloud;

    // Healthy and idle first, so the two readings can be told apart at all.
    app.load_cloud(&store);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let healthy = buffer_text(terminal.backend());
    assert!(healthy.contains("0 deferred"), "{healthy}");
    assert!(!healthy.contains("could not be read"), "{healthy}");

    // Then a store that cannot answer. Dropping the table is a stand-in for
    // every way this fails for real — a half-applied migration, a file damaged
    // under it, a lock it never got.
    store
        .connection()
        .execute_batch("DROP TABLE sync_enrolled_projects")
        .unwrap();
    app.load_cloud(&store);
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let broken = buffer_text(terminal.backend());

    assert!(
        !broken.contains("0 deferred"),
        "a count nobody could read must not be printed as a number: {broken}"
    );
    assert!(
        broken.contains("could not be read"),
        "and the reason has to be on screen: {broken}"
    );
}

/// The page says *why* replication is stopped, not only how much is waiting.
///
/// Four queued mutations look identical whether the next cycle is a minute away
/// or the server has been refusing the token for three days. The state that
/// tells them apart is persisted and `leteo cloud status` has always printed
/// it; this page — the one somebody opens *because* they think replication is
/// stuck — showed the queue depth and nothing about the reason.
#[test]
fn a_stopped_replication_says_what_stopped_it() {
    let (_temp, mut store) = store_of(1);
    let mut app = test_app();
    app.page = Page::Cloud;

    // Healthy first, so "backoff" below is a change rather than the only thing
    // this test has ever seen.
    store
        .get_sync_state(crate::cloud::CLOUD_SYNC_TARGET)
        .unwrap();
    app.load_cloud(&store);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let healthy = buffer_text(terminal.backend());
    assert!(
        !healthy.contains("401 Unauthorized"),
        "nothing has failed yet: {healthy}"
    );

    // Now a target that has given up, the way a real one does: a lifecycle, a
    // run of failures, and the message the wire actually returned.
    store
        .connection()
        .execute(
            "UPDATE sync_state SET lifecycle = 'backoff', consecutive_failures = 7,
                    backoff_until = '2026-08-04 09:00:00',
                    last_error = '401 Unauthorized: token expired'
             WHERE target_key = ?1",
            [crate::cloud::CLOUD_SYNC_TARGET],
        )
        .unwrap();
    app.load_cloud(&store);
    terminal.draw(|frame| render(frame, &app)).unwrap();
    let stuck = buffer_text(terminal.backend());

    assert!(stuck.contains("backoff"), "the state itself: {stuck}");
    assert!(
        stuck.contains("7 failure(s)"),
        "one failure and seven are different situations: {stuck}"
    );
    assert!(
        stuck.contains("401 Unauthorized"),
        "the reason is the whole point of the page: {stuck}"
    );
}
