use std::cell::Cell;
use std::io;
use std::path::Path;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::{
    memory::model::{
        Caveat, Listing, Observation, Prompt, SessionSummary, Stats, SyncState, TimelineResult,
    },
    store::{Store, StoreError},
};

mod actions;
mod app;
mod chrome;
mod views;

use actions::*;
use chrome::*;
use views::*;

/// Every screen below reads its words from the catalogue rather than carrying
/// them, so the import belongs here with the other things they all use.
use crate::i18n::fill;

/// How many rows a page of a list holds.
///
/// How many rows are read ahead of what is on screen.
///
/// Not a page, and nothing on screen calls it one. Moving through a list is
/// measured in rows and in screenfuls — the units somebody can see — and this
/// is only how far past the edge of the panel the store is read so that
/// scrolling does not ask a question per keypress. Big enough that reaching the
/// end of it is rare; small enough that changing a filter is still one quick
/// answer.
const WINDOW: usize = 400;

/// How tall a list panel is assumed to be before one has been drawn.
///
/// Only the first PgDn of a session could use it, and only if it arrived before
/// the first frame — which it cannot. It exists so the height has a value at
/// all rather than an unwrap.
const ASSUMED_HEIGHT: usize = 20;

/// Why the dashboard closed.
///
/// An uninstall cannot finish inside this process: it holds the database open,
/// and on Windows an open file cannot be deleted. So the dashboard collects the
/// agreement and hands the work back out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Normal,
    Uninstall,
}

/// Runs Leteo's interactive terminal UI on the current thread.
pub fn run(store: &mut Store) -> anyhow::Result<Exit> {
    let mut app = App::load(store)?;
    let _terminal_guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    loop {
        terminal.draw(|frame| render(frame, &app))?;
        match event::read()? {
            Event::Key(key) => {
                let action = app.handle_key(key);
                if apply_action(&mut app, store, action) {
                    break;
                }
            }
            Event::Paste(text) if app.focus == Focus::Query => app.query.push_str(&text),
            _ => {}
        }
    }

    Ok(app.exit)
}

struct TerminalGuard {
    raw_mode: bool,
    alternate_screen: bool,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        let mut guard = Self {
            raw_mode: true,
            alternate_screen: false,
        };
        enable_raw_mode()?;

        guard.alternate_screen = true;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(guard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.raw_mode {
            let _ = disable_raw_mode();
        }
        if self.alternate_screen {
            let _ = execute!(io::stdout(), LeaveAlternateScreen, cursor::Show);
        }
    }
}

/// Where a home screen entry leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuTarget {
    Open(Page),
    /// Take Leteo off the machine. Asks first, like every other removal here.
    Uninstall,
    Quit,
}

/// What the home screen offers, in the order it lists them.
///
/// Kept as data rather than as arms in the renderer and arms in the key
/// handler: the two would be edited separately and the labels would stop
/// matching what selecting them does.
///
/// The label is a function of the language rather than a string, because the
/// list is a `const` and the words are not known until the settings are read.
type MenuLabel = fn(&crate::i18n::Screens) -> &'static str;

const MENU: &[(MenuLabel, MenuTarget)] = &[
    // First, and named for what somebody came to do rather than for the page it
    // opens. Whoever has just installed the binary has nothing to browse yet,
    // and this is the only entry that gets them anywhere.
    (|say| say.menu_start_setup, MenuTarget::Open(Page::Setup)),
    // Searching, reading the recent memories and browsing sessions were three
    // entries onto three pages the dashboard has since absorbed: it holds the
    // same lists, narrows them by project and by words, and opens the same rows.
    // Three doors into one room read as three rooms.
    (|say| say.menu_dashboard, MenuTarget::Open(Page::Dashboard)),
    (|say| say.menu_cloud, MenuTarget::Open(Page::Cloud)),
    // Under the things Leteo does and above the things somebody falls back on,
    // which is where a menu puts its settings. It is not a rung on the way in —
    // the defaults are answers — so it sits with help rather than with the two
    // entries somebody opens on the day they install.
    //
    // It was third for a while, on the argument that it is the entry somebody
    // needs when they cannot read the other two. That reason went with the
    // label: the entry no longer carries the English word alongside its own, so
    // being high on the list buys a reader in the wrong language nothing.
    (|say| say.menu_options, MenuTarget::Open(Page::Options)),
    (|say| say.menu_help, MenuTarget::Open(Page::Help)),
    // Listed even though `q` also works: somebody who arrived at a full-screen
    // program has no way to know that without being told.
    (|say| say.menu_uninstall, MenuTarget::Uninstall),
    (|say| say.menu_quit, MenuTarget::Quit),
];

/// Which of the three lists the dashboard is showing.
///
/// One at a time, and it fills the screen below the panels. Showing all three at
/// once meant three short lists — the observations had four rows on a forty-row
/// terminal, which is the list somebody came to read.
///
/// The counters along the top double as its tabs: the one whose list is showing
/// is the one lit up, so what is on screen and what it is counting are the same
/// statement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Observations,
    Sessions,
    Prompts,
}

impl ListKind {
    /// The next one, in the order the counters are laid out.
    fn next(self) -> Self {
        match self {
            Self::Observations => Self::Sessions,
            Self::Sessions => Self::Prompts,
            Self::Prompts => Self::Observations,
        }
    }
}

/// Where the dashboard's keys are pointed.
///
/// Tab moves between the three lists, and those are the resting place. The other
/// two are not lists at all but ways of narrowing whichever one is showing — by
/// project, and by words — which is why they are modes reached with their own
/// key rather than further stops on the Tab cycle. Tab asks *what to read*;
/// these two ask *what to read about*, and mixing the two questions into one
/// cycle is what makes a screen feel arbitrary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Filters,
    Query,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    /// The landing screen: the drawing, the wordmark and [`MENU`].
    Home,
    Dashboard,
    Detail,
    /// One session: what it was, and everything it recorded.
    Session,
    Timeline,
    Setup,
    /// The three questions that belong to the store: what language Leteo
    /// speaks, what language memories are written in, and how much Sardi says.
    ///
    /// A page of its own rather than a detour through [`Page::Setup`] because
    /// none of it is an agent's — all three land in one `settings.json` beside
    /// the store and serve every agent pointed at it. It runs the same wizard
    /// in its other flow — see [`crate::setup::wizard::Wizard::preferences`] —
    /// so there is one renderer and one apply, and no second screen that can
    /// drift from the first.
    Options,
    Cloud,
    Help,
}

impl Page {
    fn title(self, language: crate::settings::Interface) -> &'static str {
        let say = crate::i18n::screens(language);
        match self {
            Self::Home => say.page_home,
            Self::Dashboard => say.page_dashboard,
            Self::Detail => say.page_detail,
            Self::Session => say.page_session,
            Self::Timeline => say.page_timeline,
            Self::Setup => say.page_setup,
            Self::Options => say.page_options,
            Self::Cloud => say.page_cloud,
            Self::Help => say.page_help,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    None,
    Quit,
    OpenObservation(i64),
    /// Open one session, with everything it recorded.
    OpenSession(String),
    LoadTimeline(i64),
    /// Reload every list from the store.
    ///
    /// There is no separate search action: the query is part of what the lists
    /// *are*, alongside the project filter, so running one, clearing one,
    /// ticking a project and deleting a memory all end in the same reload. A
    /// second path would be a second chance for the screen and the store to
    /// disagree — which is how the old search page kept showing rows that had
    /// already been deleted.
    Refresh,
    /// Re-read the three lists, and nothing else.
    ///
    /// What a keystroke in the query row costs. A full refresh would read the
    /// store's totals and the replication state — a file off disk — per letter
    /// typed, and neither of those changes while somebody is typing.
    Narrow,
    /// Copy text to the system clipboard.
    Copy(String),
    /// Work out what a delete would destroy, and put it up for agreement.
    ///
    /// Separate from the delete itself because the numbers have to be right, and
    /// only the store knows them. The count carried on a session row is of
    /// whatever the list was narrowed to; a project's row on the dashboard is a
    /// name and nothing else. A confirmation is worth having only if what it
    /// says is what is about to happen.
    Confirm {
        what: Target,
        hard: bool,
    },
    /// Remove something, and everything that only existed inside it.
    ///
    /// `hard` is the difference between a tombstone the store can still see and
    /// a row that is gone. Both ask first.
    Delete {
        what: Target,
        hard: bool,
    },
    /// Put removing Leteo from the machine up for agreement.
    ConfirmUninstall,
    /// Leave the dashboard so the caller can carry the uninstall out.
    ///
    /// Not done here, and the reason is the open database. This process holds
    /// `leteo.db`, and Windows refuses to delete a file that is open — so an
    /// uninstall run from inside the dashboard would take the agent
    /// configuration, fail on the store, and report a partial removal that
    /// looked like a bug. The store is dropped by the caller first.
    Uninstall,
}

/// What a delete is aimed at.
///
/// Named by the thing rather than by the list it was on, so the confirmation and
/// the store call cannot come to disagree about what is about to go.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Observation(i64),
    Prompt(i64),
    /// A session and everything it recorded.
    Session(String),
    /// A project, and every session, memory and prompt inside it.
    Project(String),
}

#[derive(Debug, Clone)]
struct StatusMessage {
    text: String,
    is_error: bool,
}

#[derive(Debug)]
struct App {
    page: Page,
    history: Vec<Page>,
    database_path: String,
    /// The language Leteo speaks, read once when the dashboard opens.
    ///
    /// Read here rather than at each line that needs it, for the reason the
    /// hooks read it once per run: a settings file saved while the dashboard is
    /// up must not leave one half of a screen in each language.
    interface: crate::settings::Interface,
    /// The language Sardi speaks here, which is [`App::interface`] unless the
    /// voice has been given one of its own.
    ///
    /// Held beside it rather than resolved at each line for the same reason:
    /// one screen must not answer half its lines from a file as it was and
    /// half from the file as it is.
    voice_interface: crate::settings::Interface,
    stats: Stats,
    recent: Vec<Observation>,
    sessions: Vec<SessionSummary>,
    prompts: Vec<Prompt>,
    detail: Option<Observation>,
    /// What the graph says about the memory on the detail page.
    ///
    /// The agent is told when a memory has been overturned — on a prompt, at a
    /// session opening, and in the reply to `mem_get_observation`. A person
    /// reading the same memory here was the one surface left that showed a
    /// stale decision as though it still held.
    detail_caveats: Vec<Caveat>,
    /// The session the session page is showing, and what it recorded.
    session: Option<(SessionSummary, Vec<Observation>)>,
    timeline: Option<TimelineResult>,
    /// The words every list on the dashboard is limited to.
    ///
    /// Empty is no search at all, and then the lists are the recent rows. It
    /// sits beside [`Self::projects_filter`] because the two are the same kind
    /// of thing — one narrows by who wrote it, the other by what it says — and
    /// they compose.
    query: String,
    recent_selected: usize,
    session_selected: usize,
    /// Which row of the open session's observations is under the cursor.
    session_entry_selected: usize,
    /// How far into each list the page in hand starts.
    ///
    /// One per list rather than one shared, so Tab comes back to where somebody
    /// left off instead of to the top of a list they had already worked down.
    recent_offset: usize,
    session_offset: usize,
    prompt_offset: usize,
    session_entry_offset: usize,
    /// How long each list is under the narrowings in force.
    ///
    /// Not the same as the store's totals, which is the point: the counters put
    /// the two side by side, and the difference is what the filters are doing.
    recent_total: i64,
    session_total: i64,
    prompt_total: i64,
    session_entry_total: i64,
    timeline_selected: usize,
    home_selected: usize,
    project_selected: usize,
    prompt_selected: usize,
    /// How many rows the list panel last had room for.
    ///
    /// Written by the renderer and read by the keys, which is why it is a
    /// [`Cell`]: PgDn means "a screenful", and only the frame that was drawn
    /// knows how much that is. Guessing a number here instead would make the
    /// key move by something other than what somebody just looked at.
    list_height: Cell<usize>,
    /// Where the setup page looks for existing agent configuration.
    ///
    /// Held rather than built where it is used, so a test can point it at a
    /// sandbox. Reading the real files from inside a unit test made the setup
    /// page's behaviour depend on which agents the developer had installed.
    setup_probe: crate::setup::SetupOptions,
    /// Which list is on screen.
    list: ListKind,
    /// Where the keys are pointed.
    focus: Focus,
    /// The projects every list on the dashboard is limited to.
    ///
    /// Empty is every project, which is what the store's queries mean by an
    /// empty set, so this is passed straight through rather than translated at
    /// each call.
    projects_filter: Vec<String>,
    /// The setup flow, while the setup page is open.
    ///
    /// The same [`crate::setup::wizard::Wizard`] `leteo setup` drives. It renders to
    /// roles rather than to a terminal, which is what lets both paint it — and
    /// what stops this page from being a second setup flow that drifts from the
    /// first.
    wizard: Option<crate::setup::wizard::Wizard>,
    detail_scroll: u16,
    status: Option<StatusMessage>,
    /// An action waiting for the user to confirm it.
    pending: Option<PendingAction>,
    cloud: CloudOverview,
    /// Why the dashboard is closing, read by [`run`] once it has.
    exit: Exit,
}

/// A destructive act waiting to be agreed to.
///
/// It is shown as a window over the middle of the screen rather than as a line
/// along the bottom. The footer is where the keys live and it is read past; this
/// is the one thing on the screen that cannot be undone, and it should take the
/// screen while it is being asked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingAction {
    /// What is about to go, named.
    heading: String,
    /// What goes with it, a line each, counted from the store.
    detail: Vec<String>,
    action: Action,
}

/// Read-only replication state shown on the cloud page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CloudOverview {
    configured: bool,
    server: String,
    enabled: bool,
    projects: Vec<String>,
    enrolled: Vec<String>,
    pending_mutations: i64,
    deferred: i64,
    dead: i64,
    /// Why the four counts above mean nothing, when they mean nothing.
    ///
    /// They are zero both when replication is idle and when the store refused
    /// to say, and those two readings call for opposite reactions. `Some` says
    /// which one this is.
    unreadable: Option<String>,
    /// What replication is doing, and why it is not doing anything.
    ///
    /// The counts say how much is waiting. They cannot say whether it is
    /// waiting because the next cycle is a minute away or because sync has been
    /// in backoff for three days on an expired token — and those look
    /// identical: four queued mutations either way.
    ///
    /// The state is persisted and `leteo cloud status` has always printed it.
    /// This page, which is the one somebody opens *because* they think
    /// replication is stuck, showed the queue depth and not the reason.
    state: Option<SyncState>,
}

fn clamp_index(selected: usize, len: usize) -> usize {
    if len == 0 { 0 } else { selected.min(len - 1) }
}

/// Where a window that ends at the end of a list of this length begins.
fn last_window(total: i64) -> usize {
    usize::try_from(total).unwrap_or(0).saturating_sub(WINDOW)
}

/// Fetches a page, stepping back if the offset has fallen off the end.
///
/// Deleting the last row, or narrowing a list until it is shorter than where
/// somebody had scrolled to, leaves the offset pointing past everything — and a
/// window read from there comes back empty, which reads as "nothing found" when
/// the answer is "you are past the end".
fn page_of<T>(
    offset: &mut usize,
    fetch: impl Fn(usize) -> Result<Listing<T>, StoreError>,
) -> Result<Listing<T>, StoreError> {
    let listing = fetch(*offset)?;
    if !listing.rows.is_empty() || *offset == 0 {
        return Ok(listing);
    }
    *offset = last_window(listing.total);
    fetch(*offset)
}

/// One counter, and a tab for the list it counts.
fn render_stat(frame: &mut Frame<'_>, area: Rect, title: &str, value: &str, showing: bool) {
    let widget = Paragraph::new(value.to_owned())
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(if showing { Color::Cyan } else { Color::Gray })
                .add_modifier(Modifier::BOLD),
        )
        .block(focus_panel(title, showing));
    frame.render_widget(widget, area);
}

#[cfg(test)]
mod tests;
