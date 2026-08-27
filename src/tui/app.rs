//! Everything the dashboard does in response to a key.
//!
//! The struct itself stays in the parent: its fields are private, and a
//! sibling module cannot see another sibling's privates — only a child can
//! see its ancestor's. Moving the state here would have meant making every
//! field `pub(super)` for the sake of a file boundary.

use super::*;

impl App {
    pub(super) fn load(store: &mut Store) -> Result<Self, StoreError> {
        let settings = crate::settings::load_beside(store.database_path());
        let mut app = Self {
            // The TUI opens on the home screen, not on somebody's data.
            page: Page::Home,
            history: Vec::new(),
            database_path: store.database_path().display().to_string(),
            interface: settings.interface(),
            voice_interface: settings.voice_language(),
            stats: store.stats()?,
            recent: Vec::new(),
            sessions: Vec::new(),
            prompts: Vec::new(),
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
            recent_total: 0,
            session_total: 0,
            prompt_total: 0,
            session_entry_total: 0,
            timeline_selected: 0,
            home_selected: 0,
            project_selected: 0,
            prompt_selected: 0,
            setup_probe: crate::setup::SetupOptions::default(),
            list_height: Cell::new(ASSUMED_HEIGHT),
            list: ListKind::Observations,
            focus: Focus::List,
            projects_filter: Vec::new(),
            wizard: None,
            detail_scroll: 0,
            status: None,
            pending: None,
            cloud: CloudOverview::default(),
            exit: Exit::Normal,
        };
        // The lists start empty and are filled by the one path that fills them,
        // rather than by a second set of queries here that would have to be kept
        // saying the same thing as `refresh` does.
        app.refresh(store)?;
        Ok(app)
    }

    /// Reloads the three lists, and nothing else.
    ///
    /// What a keypress in the query row costs. The store's totals and the
    /// replication state do not change while somebody is typing, and reading
    /// them per keystroke — the cloud state means a file off disk — is what
    /// would make a search that runs as it is typed feel slow.
    pub(super) fn narrow(&mut self, store: &Store) -> Result<(), StoreError> {
        let filter = self.projects_filter.clone();
        let query = self.query.trim().to_owned();
        let recent = page_of(&mut self.recent_offset, |at| {
            store.paged_observations(&query, &filter, at, WINDOW)
        })?;
        let sessions = page_of(&mut self.session_offset, |at| {
            store.paged_sessions(&query, &filter, at, WINDOW)
        })?;
        let prompts = page_of(&mut self.prompt_offset, |at| {
            store.paged_prompts(&query, &filter, at, WINDOW)
        })?;
        self.recent = recent.rows;
        self.recent_total = recent.total;
        self.sessions = sessions.rows;
        self.session_total = sessions.total;
        self.prompts = prompts.rows;
        self.prompt_total = prompts.total;
        self.clamp_selections();
        Ok(())
    }

    /// Everything: the lists, the store's totals, the open session, the cloud.
    ///
    /// Nothing is assigned until every query has come back, so a query the store
    /// refuses leaves the previous screen intact with the complaint on the
    /// status line rather than blanking three lists to report one mistake.
    pub(super) fn refresh(&mut self, store: &mut Store) -> Result<(), StoreError> {
        // Stats stay global on purpose: they are the totals of the store, and a
        // filtered count under a heading that says OBSERVATIONS would read as
        // the store having shrunk. What is narrowed is said beside them instead,
        // as a count of matches.
        let stats = store.stats()?;
        self.narrow(store)?;
        self.stats = stats;
        // The open session is reloaded too, because turning its page is a
        // refresh like any other and it is not one of the three above.
        if let Some((summary, _)) = &self.session {
            let id = summary.id.clone();
            let entries = page_of(&mut self.session_entry_offset, |at| {
                store.paged_session_observations(&id, at, WINDOW)
            })?;
            self.session_entry_total = entries.total;
            if let Some((_, rows)) = self.session.as_mut() {
                *rows = entries.rows;
            }
        }
        self.load_cloud(store);
        Ok(())
    }

    /// Reads replication state.
    ///
    /// Two different failures, and only one of them is nothing to worry about.
    /// The config file is genuinely optional — not finding one means cloud sync
    /// was never set up, which the page says plainly.
    ///
    /// The counts underneath are not optional in the same way. They used to
    /// default on error, so a store that could not answer produced "none
    /// enrolled, 0 queued, 0 deferred, 0 dead" — which is also exactly what a
    /// perfectly healthy idle store produces. This is the page somebody opens
    /// *because* they suspect replication is stuck, and it was answering a
    /// question it had failed to ask.
    pub(super) fn load_cloud(&mut self, store: &Store) {
        let mut overview = CloudOverview::default();
        if let Some(directory) = store.database_path().parent() {
            let path = crate::cloud::ClientConfig::path_in(directory);
            if let Ok(config) = crate::cloud::ClientConfig::load(&path) {
                overview.configured = !config.server.is_empty();
                overview.server = config.server;
                overview.enabled = config.enabled;
                overview.projects = config.projects;
            }
        }
        // Read even when the counts below fail: the reason a queue is stuck is
        // exactly what a store too busy to count is likeliest to be stuck on.
        overview.state = store
            .sync_state_if_any(crate::cloud::CLOUD_SYNC_TARGET)
            .ok()
            .flatten();
        match Self::replication_state(store) {
            Ok((enrolled, pending, deferred, dead)) => {
                overview.enrolled = enrolled;
                overview.pending_mutations = pending;
                overview.deferred = deferred;
                overview.dead = dead;
            }
            Err(error) => overview.unreadable = Some(error.to_string()),
        }
        self.cloud = overview;
    }

    /// The four replication counts, or the first reason there are none.
    ///
    /// Together rather than one by one: they describe one queue, and a panel
    /// showing three real numbers beside one silent zero is worse than a panel
    /// that admits it could not read the queue at all.
    fn replication_state(
        store: &Store,
    ) -> Result<(Vec<String>, i64, i64, i64), crate::store::StoreError> {
        let enrolled = store.enrolled_projects()?;
        let pending = store.pending_sync_mutation_count(crate::cloud::CLOUD_SYNC_TARGET)?;
        let (deferred, dead) = store.deferred_sync_counts()?;
        Ok((enrolled, pending, deferred, dead))
    }

    /// The observation the current page has selected, if any.
    ///
    /// This is what `y` copies and what `d` deletes, so it has to be a row
    /// somebody can see. The dashboard qualifies only while the observations
    /// list is the one showing: with sessions on screen, the observation cursor
    /// is still sitting somewhere off it, and deleting whatever it happens to
    /// be resting on is not what pressing `d` meant.
    fn selected_observation(&self) -> Option<&Observation> {
        match self.page {
            Page::Dashboard if self.list == ListKind::Observations && self.focus == Focus::List => {
                self.recent.get(self.recent_selected)
            }
            Page::Session => self
                .session
                .as_ref()
                .and_then(|(_, entries)| entries.get(self.session_entry_selected)),
            Page::Detail => self.detail.as_ref(),
            _ => None,
        }
    }

    /// What `d` and `D` are aimed at, given where the cursor is.
    ///
    /// Whatever is under it and on screen. On the observations list that is a
    /// memory, on the sessions list a session, on the prompts list a prompt, and
    /// in the filter panel the project whose box is highlighted — so the key
    /// means the same thing everywhere it is pressed: remove this.
    fn delete_target(&self) -> Option<Target> {
        if self.page == Page::Dashboard && self.focus == Focus::Filters {
            return self
                .stats
                .projects
                .get(self.project_selected)
                .cloned()
                .map(Target::Project);
        }
        if self.page == Page::Dashboard && self.focus == Focus::List {
            match self.list {
                ListKind::Sessions => {
                    return self
                        .sessions
                        .get(self.session_selected)
                        .map(|session| Target::Session(session.id.clone()));
                }
                ListKind::Prompts => {
                    return self
                        .prompts
                        .get(self.prompt_selected)
                        .map(|prompt| Target::Prompt(prompt.id));
                }
                ListKind::Observations => {}
            }
        }
        self.selected_observation()
            .map(|observation| Target::Observation(observation.id))
    }

    fn handle_confirmation_key(&mut self, key: KeyEvent) -> Action {
        // Only a yes goes through, and Enter is not one of them. A window that
        // takes the screen catches whatever was being pressed when it appeared,
        // and Enter is the key somebody was most likely already pressing.
        let is_yes = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));
        let Some(pending) = self.pending.take() else {
            return Action::None;
        };
        if is_yes {
            return pending.action;
        }
        self.status = Some(StatusMessage {
            text: crate::i18n::screens(self.interface).cancelled.to_owned(),
            is_error: false,
        });
        Action::None
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        if self.pending.is_some() {
            return self.handle_confirmation_key(key);
        }
        // The query line takes every key while it has them, or `q` would quit
        // halfway through typing "query".
        if self.focus == Focus::Query {
            return self.handle_query_key(key);
        }
        // The wizard owns every key while it is open. Letting the shared
        // handler through would mean `s` jumped to Sessions from the middle of a
        // setup flow, which is not something anybody intends.
        if matches!(self.page, Page::Setup | Page::Options) {
            return self.handle_wizard_key(key);
        }

        match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('y') => match self.selected_observation() {
                Some(observation) => {
                    Action::Copy(format!("{}\n\n{}", observation.title, observation.content))
                }
                None => Action::None,
            },
            KeyCode::Char('d') => match self.delete_target() {
                Some(what) => Action::Confirm { what, hard: false },
                None => Action::None,
            },
            KeyCode::Char('D') => match self.delete_target() {
                Some(what) => Action::Confirm { what, hard: true },
                None => Action::None,
            },
            KeyCode::Char('S') => {
                self.navigate(Page::Setup);
                Action::None
            }
            KeyCode::Char('c') => {
                self.navigate(Page::Cloud);
                Action::None
            }
            KeyCode::Char('/') => {
                self.begin_query();
                Action::None
            }
            KeyCode::Char('?') | KeyCode::Char('h') => {
                self.navigate(Page::Help);
                Action::None
            }
            KeyCode::Char('g') => {
                self.show_list(ListKind::Observations);
                Action::None
            }
            KeyCode::Char('r') => {
                self.show_list(ListKind::Observations);
                Action::None
            }
            KeyCode::Char('R') => Action::Refresh,
            KeyCode::Char('s') => {
                self.show_list(ListKind::Sessions);
                Action::None
            }
            KeyCode::Char('t') if self.page == Page::Detail => {
                self.detail.as_ref().map_or(Action::None, |observation| {
                    Action::LoadTimeline(observation.id)
                })
            }
            KeyCode::Tab if self.page == Page::Dashboard => {
                self.cycle_list();
                Action::None
            }
            // `f` rather than `p`: the panel started as a list of projects and
            // is now where every narrowing is shown, the query included.
            KeyCode::Char('f') if self.page == Page::Dashboard => {
                self.toggle_filters();
                Action::None
            }
            KeyCode::Char(' ') if self.focus == Focus::Filters && self.page == Page::Dashboard => {
                self.toggle_project();
                // The lists come from the store, so a filter is a query rather
                // than a view over what is already in hand.
                Action::Refresh
            }
            // Esc undoes the smallest thing it can, and these are in the order
            // they were put in place: leave the filter panel, then drop the
            // search, then leave the page.
            KeyCode::Esc if self.page == Page::Dashboard && self.focus == Focus::Filters => {
                self.focus = Focus::List;
                Action::None
            }
            KeyCode::Esc if self.page == Page::Dashboard && !self.query.is_empty() => {
                self.clear_query()
            }
            KeyCode::Esc => {
                self.back();
                Action::None
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            // A screenful at a time, and the ends of the list. A screenful is
            // what was just looked at, so these move by something somebody can
            // see rather than by a number chosen in here.
            KeyCode::PageDown if self.scrolls() => self.move_selection(self.screenful()),
            KeyCode::PageUp if self.scrolls() => self.move_selection(-self.screenful()),
            // Far enough to reach either end of any list, and `step` clamps it
            // to the row that is actually there.
            KeyCode::End if self.scrolls() => self.move_selection(isize::MAX),
            KeyCode::Home if self.scrolls() => self.move_selection(isize::MIN),
            KeyCode::Enter => self.activate_selection(),
            _ => Action::None,
        }
    }

    /// Drives the setup wizard, with the same keys `leteo setup` uses.
    fn handle_wizard_key(&mut self, key: KeyEvent) -> Action {
        let Some(wizard) = self.wizard.as_mut() else {
            self.back();
            return Action::None;
        };
        match key.code {
            KeyCode::Char('q') => return Action::Quit,
            KeyCode::Char('j') | KeyCode::Down => wizard.down(),
            KeyCode::Char('k') | KeyCode::Up => wizard.up(),
            KeyCode::Char(' ') => wizard.toggle(),
            KeyCode::Enter => wizard.advance(),
            KeyCode::Backspace => wizard.back(),
            KeyCode::Esc => wizard.cancel(),
            _ => false,
        };

        if !wizard.is_finished() {
            // The language the page is painted in can be changed by the page
            // itself, and the window around it — the header, the border, the
            // footer — is painted from this copy. Read back on every key so the
            // whole screen turns at once. Otherwise picking a language repaints
            // the panel and leaves the frame around it in the old one, which
            // reads as a setting that only half took.
            self.interface = wizard.interface();
            self.voice_interface = wizard.voice_interface();
            return Action::None;
        }
        // Finishing the flow is what applies it, exactly as it does for
        // `leteo setup`. The steps are the confirmation: nothing is written
        // until somebody has ticked what they wanted and pressed enter through
        // to the end.
        let wizard = self.wizard.take().expect("checked above");
        let mut report = Vec::new();
        let applied = wizard.apply(&mut report);
        // Re-read before a word of this is chosen. The flow that just finished
        // is the one that can change what language every screen here is painted
        // in, and `interface` was read once when the dashboard opened — so the
        // setting used to take effect on the next start, which reads as a
        // setting that did not take. The wizard itself repaints as the answer is
        // picked, which made the rest of the window the odd one out.
        //
        // Including the sentence below, which the wizard has already written
        // into `report` in the new language: composing the failure around it in
        // the old one would put two languages in one line.
        let settings = crate::settings::load_beside(Path::new(&self.database_path));
        self.interface = settings.interface();
        self.voice_interface = settings.voice_language();
        let say = crate::i18n::screens(self.interface);
        // Whether this failed is decided here, where it is known, rather than by
        // reading the sentence back afterwards.
        //
        // It used to be `text.starts_with("setup failed")` — a branch keyed on
        // the display text, which is the one thing about a message that is
        // allowed to change. Translating that string, or rewording it, would
        // have painted every failure in the ordinary colour and gone on
        // reporting a successful setup that never happened.
        let (text, failed) = match applied {
            Ok(outcome) if outcome.cancelled => (say.setup_cancelled.to_owned(), false),
            Ok(_) => (
                String::from_utf8_lossy(&report).trim().replace('\n', "  "),
                false,
            ),
            Err(error) => (crate::i18n::fill(say.setup_failed, "error", error), true),
        };
        self.status = Some(StatusMessage {
            is_error: failed,
            text,
        });
        self.back();
        Action::Refresh
    }

    /// Types the query the lists are narrowed by.
    ///
    /// Every keystroke applies, the way ticking a project does. Making one
    /// narrowing take effect at once and the other wait for Enter meant two
    /// things that do the same job behaving differently on the same screen, and
    /// the counters along the top are worth most while the word is still being
    /// typed — they say which list the answer is in before it is finished.
    ///
    /// The store matches the last word by prefix for exactly this reason, so
    /// `postgr` finds `postgres` rather than reading as nothing found.
    fn handle_query_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            // Both leave the input, and neither undoes the search: it has been
            // in force since the first letter. Esc from the list is what drops
            // it — the smallest thing first, then the next.
            KeyCode::Esc | KeyCode::Enter => {
                self.focus = Focus::List;
                Action::None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.narrowed()
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.clear();
                self.narrowed()
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(character);
                self.narrowed()
            }
            _ => Action::None,
        }
    }

    /// The query changed: back to the top of every list, and re-read them.
    ///
    /// To the top because the rows are not the rows any more. Row eleven of the
    /// old results is not row eleven of the new ones, and leaving the cursor
    /// where it was means it lands on whatever happens to be there.
    fn narrowed(&mut self) -> Action {
        self.reset_cursors();
        Action::Narrow
    }

    /// Puts the keys in the query line, on the dashboard.
    ///
    /// `/` works from anywhere, so from another page this also gets there:
    /// searching is something the dashboard does, and a key that quietly did
    /// nothing on four pages out of eight would read as broken.
    fn begin_query(&mut self) {
        // An empty store shows the cat and a sentence instead of the panels, so
        // there is no row to type on. Putting the keys in an input that is not
        // drawn is a screen nothing responds to.
        if self.stats.total_observations == 0 {
            self.set_error(crate::i18n::screens(self.interface).nothing_to_search);
            return;
        }
        if self.page != Page::Dashboard {
            self.navigate(Page::Dashboard);
        }
        self.focus = Focus::Query;
        self.status = None;
    }

    /// Drops the search and puts the recent lists back.
    fn clear_query(&mut self) -> Action {
        self.query.clear();
        self.focus = Focus::List;
        self.reset_cursors();
        Action::Refresh
    }

    /// Shows the dashboard, with a chosen list on it.
    ///
    /// What `g`, `r` and `s` do now that the pages they opened are tabs here.
    fn show_list(&mut self, list: ListKind) {
        self.navigate(Page::Dashboard);
        self.list = list;
        self.focus = Focus::List;
    }

    /// Sends every list cursor back to the top.
    ///
    /// Called whenever the rows change underneath it. Row eleven of the old
    /// results is not row eleven of the new ones, and leaving the cursor there
    /// means the highlight lands somewhere nobody chose.
    fn reset_cursors(&mut self) {
        self.recent_selected = 0;
        self.session_selected = 0;
        self.prompt_selected = 0;
        // And back to the first page: page four of a search nobody is running
        // any more is not a place to land.
        self.recent_offset = 0;
        self.session_offset = 0;
        self.prompt_offset = 0;
    }

    pub(super) fn navigate(&mut self, page: Page) {
        if self.page != page {
            self.history.push(self.page);
            self.page = page;
        }
        self.focus = Focus::List;
        // A fresh wizard each time the page is opened. Resuming a half-finished
        // one would show ticks somebody set minutes ago and has forgotten.
        //
        // Two pages, one flow: the setup page runs it from the top and the
        // language page enters it at the global half. Everything below —
        // keys, painting, applying — reads `self.wizard` and never asks which
        // page it came from, so the two cannot answer a key differently.
        self.wizard = match page {
            Page::Setup | Page::Options => {
                let offer = crate::setup::wizard::offer(
                    Path::new(&self.database_path),
                    self.stats.total_observations == 0,
                    &self.setup_probe,
                );
                Some(if page == Page::Setup {
                    crate::setup::wizard::Wizard::new(offer)
                } else {
                    crate::setup::wizard::Wizard::preferences(offer)
                })
            }
            _ => None,
        };
    }

    pub(super) fn back(&mut self) {
        self.page = self.history.pop().unwrap_or(Page::Home);
        self.focus = Focus::List;
    }

    /// Moves the cursor, and says whether the rows have to be re-read.
    ///
    /// Stepping off the end of a page is how the next one is reached, so the
    /// arrow keys walk the whole list rather than stopping at row one hundred
    /// of three thousand. That crossing is the only thing here that needs the
    /// store, which is what the returned action says.
    pub(super) fn move_selection(&mut self, delta: isize) -> Action {
        match self.page {
            Page::Dashboard if self.focus == Focus::Filters => {
                // Counted before the borrow: the length reads `self`, and
                // `move_index` holds a mutable borrow of a field of it.
                let count = self.stats.projects.len();
                Self::move_index(&mut self.project_selected, count, delta);
                Action::None
            }
            // The query line has no rows to move through, and the arrow keys
            // reach it through `handle_query_key` in any case.
            Page::Dashboard if self.focus == Focus::Query => Action::None,
            Page::Dashboard | Page::Session => {
                let height = self.list_height.get().max(1);
                let (selected, offset, len, total) = self.place();
                if Self::step(selected, offset, len, total, height, delta) {
                    Action::Refresh
                } else {
                    Action::None
                }
            }
            Page::Timeline => {
                let len = self.timeline.as_ref().map_or(0, |timeline| {
                    timeline.before.len() + 1 + timeline.after.len()
                });
                Self::move_index(&mut self.timeline_selected, len, delta);
                Action::None
            }
            // The setup page is the wizard, and it takes keys through
            // `handle_wizard_key` rather than through the shared selection.
            Page::Setup | Page::Options | Page::Cloud | Page::Help => Action::None,
            Page::Detail => {
                self.detail_scroll = if delta > 0 {
                    self.detail_scroll.saturating_add(1)
                } else {
                    self.detail_scroll.saturating_sub(1)
                };
                Action::None
            }
            Page::Home => {
                Self::move_index(&mut self.home_selected, MENU.len(), delta);
                Action::None
            }
        }
    }

    /// The cursor, the page start, the rows in hand and the length of the whole
    /// list — for whichever list the keys are on.
    ///
    /// One place rather than a `match` repeated in every mover: the three lists
    /// and the session's differ in which fields they use and in nothing else,
    /// and a second copy of that mapping is a second chance for one of them to
    /// move a cursor that belongs to another.
    fn place(&mut self) -> (&mut usize, &mut usize, usize, i64) {
        if self.page == Page::Session {
            let len = self
                .session
                .as_ref()
                .map_or(0, |(_, entries)| entries.len());
            return (
                &mut self.session_entry_selected,
                &mut self.session_entry_offset,
                len,
                self.session_entry_total,
            );
        }
        match self.list {
            ListKind::Observations => (
                &mut self.recent_selected,
                &mut self.recent_offset,
                self.recent.len(),
                self.recent_total,
            ),
            ListKind::Sessions => (
                &mut self.session_selected,
                &mut self.session_offset,
                self.sessions.len(),
                self.session_total,
            ),
            ListKind::Prompts => (
                &mut self.prompt_selected,
                &mut self.prompt_offset,
                self.prompts.len(),
                self.prompt_total,
            ),
        }
    }

    /// Moves the cursor by `delta` rows, reading further into the list when it
    /// walks past what is in hand.
    ///
    /// Returns whether the store has to answer for it. Inside the rows already
    /// read — which is nearly always — it does not, so scrolling is a cursor
    /// moving rather than a question per keypress.
    ///
    /// When the cursor does walk out of the window, the new one is anchored so
    /// the row moved to lands at the edge of the panel it arrived from, and the
    /// rest is read out ahead in the direction of travel. Going down that makes
    /// the crossing invisible: the panel shifts by exactly the one row it would
    /// have shifted anyway.
    pub(super) fn step(
        selected: &mut usize,
        offset: &mut usize,
        len: usize,
        total: i64,
        height: usize,
        delta: isize,
    ) -> bool {
        // Never shorter than the rows in hand. Those are on screen, and a total
        // that has fallen behind them would make the cursor refuse to move onto
        // a row somebody can see.
        let total = usize::try_from(total).unwrap_or(0).max(*offset + len);
        if total == 0 || len == 0 {
            return false;
        }
        let row = *offset + *selected;
        let target = if delta >= 0 {
            row.saturating_add(delta.unsigned_abs()).min(total - 1)
        } else {
            row.saturating_sub(delta.unsigned_abs())
        };
        if target == row {
            return false;
        }
        if target >= *offset && target < *offset + len {
            *selected = target - *offset;
            return false;
        }
        let ahead = WINDOW.saturating_sub(height);
        *offset = if target > row {
            target.saturating_sub(height.saturating_sub(1))
        } else {
            target.saturating_sub(ahead)
        };
        *selected = target - *offset;
        true
    }

    fn scrolls(&self) -> bool {
        matches!(self.page, Page::Dashboard | Page::Session)
    }

    /// How many rows one press of PgDn covers: what the panel last held.
    fn screenful(&self) -> isize {
        isize::try_from(self.list_height.get().max(1)).unwrap_or(1)
    }

    /// Shows the next list.
    ///
    /// Leaves the filter panel if that is where the keys were: Tab means "show
    /// me the next list", and it should do that from wherever somebody is.
    fn cycle_list(&mut self) {
        self.focus = Focus::List;
        self.list = self.list.next();
    }

    fn toggle_filters(&mut self) {
        self.focus = if self.focus == Focus::Filters {
            Focus::List
        } else {
            Focus::Filters
        };
    }

    /// Marks or unmarks the project under the cursor.
    ///
    /// Several may be marked at once: asking for `leteo` and `engram` together
    /// is a question somebody has, and the store can answer it in one query.
    fn toggle_project(&mut self) {
        let Some(project) = self.stats.projects.get(self.project_selected).cloned() else {
            return;
        };
        match self.projects_filter.iter().position(|p| *p == project) {
            Some(at) => {
                self.projects_filter.remove(at);
            }
            None => self.projects_filter.push(project),
        }
        self.reset_cursors();
    }

    fn move_index(selected: &mut usize, len: usize, delta: isize) {
        if len == 0 {
            *selected = 0;
        } else if delta > 0 {
            *selected = (*selected + 1).min(len - 1);
        } else {
            *selected = selected.saturating_sub(1);
        }
    }

    pub(super) fn activate_selection(&mut self) -> Action {
        match self.page {
            // Enter in the filter panel does what space does, so whichever key
            // somebody reaches for works.
            Page::Dashboard if self.focus == Focus::Filters => {
                self.toggle_project();
                Action::Refresh
            }
            Page::Dashboard if self.list == ListKind::Sessions => self
                .sessions
                .get(self.session_selected)
                .map_or(Action::None, |session| {
                    Action::OpenSession(session.id.clone())
                }),
            Page::Dashboard if self.list == ListKind::Prompts => {
                // A prompt runs to paragraphs and the panel shows one line, so
                // opening it means putting the whole thing where it can be read.
                if let Some(prompt) = self.prompts.get(self.prompt_selected) {
                    self.status = Some(StatusMessage {
                        text: format!("Prompt #{}: {}", prompt.id, prompt.content.trim()),
                        is_error: false,
                    });
                }
                Action::None
            }
            Page::Dashboard => self
                .recent
                .get(self.recent_selected)
                .map_or(Action::None, |observation| {
                    Action::OpenObservation(observation.id)
                }),
            Page::Detail => self.detail.as_ref().map_or(Action::None, |observation| {
                Action::LoadTimeline(observation.id)
            }),
            Page::Timeline => self
                .selected_timeline_id()
                .map_or(Action::None, Action::OpenObservation),
            Page::Session => self
                .selected_observation()
                .map_or(Action::None, |entry| Action::OpenObservation(entry.id)),
            // Handled by `handle_wizard_key`, which owns every key on these.
            Page::Setup | Page::Options => Action::None,
            Page::Cloud => Action::None,
            Page::Help => {
                self.back();
                Action::None
            }
            Page::Home => match MENU.get(self.home_selected).map(|(_, target)| *target) {
                Some(MenuTarget::Quit) => Action::Quit,
                Some(MenuTarget::Uninstall) => Action::ConfirmUninstall,
                Some(MenuTarget::Open(page)) => {
                    self.navigate(page);
                    Action::None
                }
                // A menu index past the end cannot happen, and doing nothing is
                // the right answer if it ever does.
                None => Action::None,
            },
        }
    }

    pub(super) fn selected_timeline_id(&self) -> Option<i64> {
        let timeline = self.timeline.as_ref()?;
        let before_len = timeline.before.len();
        if self.timeline_selected < before_len {
            return Some(timeline.before[self.timeline_selected].id);
        }
        if self.timeline_selected == before_len {
            return Some(timeline.focus.id);
        }
        timeline
            .after
            .get(self.timeline_selected.saturating_sub(before_len + 1))
            .map(|entry| entry.id)
    }

    pub(super) fn open_detail(&mut self, observation: Observation, caveats: Vec<Caveat>) {
        self.navigate(Page::Detail);
        self.detail = Some(observation);
        self.detail_caveats = caveats;
        self.detail_scroll = 0;
        self.status = None;
    }

    pub(super) fn open_timeline(&mut self, timeline: TimelineResult) {
        self.navigate(Page::Timeline);
        self.timeline_selected = timeline.before.len();
        self.timeline = Some(timeline);
        self.status = None;
    }

    pub(super) fn open_session(&mut self, session: SessionSummary, entries: Listing<Observation>) {
        self.navigate(Page::Session);
        self.session = Some((session, entries.rows));
        self.session_entry_total = entries.total;
        self.session_entry_selected = 0;
        self.status = None;
    }

    /// Drops what a delete took, from the places a reload does not reach.
    ///
    /// The three dashboard lists come back from the store, so they need nothing.
    /// The open detail and the open session are copies held here, and a page
    /// showing a memory that no longer exists is a page whose every key fails.
    pub(super) fn forget(&mut self, what: &Target) {
        match what {
            Target::Observation(id) => {
                if self.detail.as_ref().is_some_and(|item| item.id == *id) {
                    self.detail = None;
                    self.back();
                }
                if let Some((_, entries)) = self.session.as_mut() {
                    let before = entries.len();
                    entries.retain(|entry| entry.id != *id);
                    if entries.len() != before {
                        self.session_entry_total -= 1;
                    }
                }
            }
            Target::Session(id) => {
                if self.session.as_ref().is_some_and(|(s, _)| s.id == *id) {
                    self.session = None;
                    self.session_entry_total = 0;
                    self.back();
                }
            }
            // A project takes sessions and memories with it, and any of them
            // could be what is open behind the dashboard. Rather than work out
            // which, the pages that show one copy are closed.
            Target::Project(_) => {
                self.detail = None;
                self.session = None;
                self.session_entry_total = 0;
                self.projects_filter.clear();
                self.history
                    .retain(|page| !matches!(page, Page::Detail | Page::Session | Page::Timeline));
                if matches!(self.page, Page::Detail | Page::Session | Page::Timeline) {
                    self.page = Page::Dashboard;
                }
                self.reset_cursors();
            }
            Target::Prompt(_) => {}
        }
    }

    pub(super) fn set_error(&mut self, error: impl std::fmt::Display) {
        self.status = Some(StatusMessage {
            text: error.to_string(),
            is_error: true,
        });
    }

    fn clamp_selections(&mut self) {
        self.recent_selected = clamp_index(self.recent_selected, self.recent.len());
        self.session_selected = clamp_index(self.session_selected, self.sessions.len());
        self.prompt_selected = clamp_index(self.prompt_selected, self.prompts.len());
    }
}
