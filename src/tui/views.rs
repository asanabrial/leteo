use super::*;

/// What a list panel is called, where the cursor is in it, and whether it holds
/// the keys.
///
/// Gathered into one value because the three renderers below take the same four
/// things and had grown to eight parameters each. Four of them describe the
/// *frame* around the list and two describe the list itself, which is the seam
/// worth cutting on.
pub(super) struct ListChrome<'a> {
    pub language: crate::settings::Interface,
    pub title: &'a str,
    pub position: &'a str,
    pub focused: bool,
}

pub(super) fn render_dashboard(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let say = crate::i18n::screens(app.interface);
    // Nothing stored yet. Four zeros and three empty lists tell someone their
    // setup is broken when it is only new, so the whole dashboard gives way to
    // Sardi and a sentence about what happens next.
    if app.stats.total_observations == 0 && app.recent.is_empty() && app.sessions.is_empty() {
        render_empty_dashboard(frame, app, area);
        return;
    }
    // The top band is five rows for four numbers, which is all the counters need
    // and nowhere near enough for a list of projects: a store with sixteen of
    // them showed three. So while the filter panel has the arrow keys, the band
    // grows to hold as many as will fit in half the screen, and the list below
    // gives up the rows. The shift is the point — it says where the keys are as
    // plainly as the border colour does.
    let band = if app.focus == Focus::Filters {
        let wanted = u16::try_from(app.stats.projects.len()).unwrap_or(u16::MAX) + 2;
        wanted.clamp(5, area.height / 2)
    } else {
        5
    };
    // The query line is always drawn, empty or not. It was hidden until there
    // was something to show — one row saved — and the result was a search
    // nobody could find: the only thing saying it existed was a word in the
    // footer, on a screen that has no other input on it. A dim `/ search` is
    // what makes the key discoverable, and one row is what it costs.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(band),
            Constraint::Length(1),
            Constraint::Min(4),
        ])
        .split(area);
    let stats = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(rows[0]);

    // The counters are the tabs. Lighting the one whose list is showing makes
    // the number and the rows below it one statement rather than two.
    //
    // With a search running they say how many of each matched, and that is what
    // turns Tab from "show me the next list" into an answer: the store says up
    // front that what was asked for is in the prompts and not in the
    // observations, before anything has been read.
    let showing = |kind: ListKind| app.focus == Focus::List && app.list == kind;
    // Two numbers only when they differ. Nothing narrowed means nothing to say
    // beyond the total, and `3312 / 3312` is a sum somebody has to do to learn
    // that no filter is in force.
    let count = |narrowed: i64, total: i64| {
        if narrowed == total {
            total.to_string()
        } else {
            format!("{narrowed} / {total}")
        }
    };
    render_stat(
        frame,
        stats[0],
        say.stat_observations,
        &count(app.recent_total, app.stats.total_observations),
        showing(ListKind::Observations),
    );
    render_stat(
        frame,
        stats[1],
        say.stat_sessions,
        &count(app.session_total, app.stats.total_sessions),
        showing(ListKind::Sessions),
    );
    render_stat(
        frame,
        stats[2],
        say.stat_prompts,
        &count(app.prompt_total, app.stats.total_prompts),
        showing(ListKind::Prompts),
    );
    render_filters(frame, app, stats[3]);

    render_query(frame, app, rows[1]);

    // The list says what it is limited to. A filtered list under a plain title
    // reads as the whole store having shrunk.
    let scope = match app.projects_filter.as_slice() {
        [] => " ".to_owned(),
        [one] => fill(say.scope_one_project, "project", one),
        // Naming them all would push the title past the panel; the count says
        // enough, and the marks in the filter panel say which.
        many => fill(say.scope_many_projects, "count", many.len()),
    };
    // A search reorders the list by how well each row matches, so it stops being
    // a record of what happened lately and the heading has to stop claiming it.
    let sense = if app.query.trim().is_empty() {
        "".to_owned()
    } else {
        fill(say.list_matching, "query", app.query.trim())
    };
    // One list, the full height of what is left. The keys are on it unless
    // somebody has stepped into the filter panel or the query line.
    let holds_keys = app.focus == Focus::List;
    // Minus the two border rows: what PgDn moves by has to be what somebody can
    // see, not what the panel occupies.
    let height = usize::from(rows[2].height.saturating_sub(2));
    app.list_height.set(height.max(1));
    match app.list {
        ListKind::Observations => render_observation_list(
            frame,
            rows[2],
            ListChrome {
                language: app.interface,
                title: &format!("{}{sense}{scope}", say.list_observations),
                position: &position(
                    app.interface,
                    app.recent_offset,
                    app.recent_selected,
                    height,
                    app.recent_total,
                ),
                focused: holds_keys,
            },
            &app.recent,
            app.recent_selected,
        ),
        ListKind::Sessions => render_session_list(
            frame,
            rows[2],
            ListChrome {
                language: app.interface,
                title: &format!("{}{sense}{scope}", say.list_sessions),
                position: &position(
                    app.interface,
                    app.session_offset,
                    app.session_selected,
                    height,
                    app.session_total,
                ),
                focused: holds_keys,
            },
            &app.sessions,
            app.session_selected,
        ),
        ListKind::Prompts => render_prompt_list(
            frame,
            rows[2],
            ListChrome {
                language: app.interface,
                title: &format!("{}{sense}{scope}", say.list_prompts),
                position: &position(
                    app.interface,
                    app.prompt_offset,
                    app.prompt_selected,
                    height,
                    app.prompt_total,
                ),
                focused: holds_keys,
            },
            &app.prompts,
            app.prompt_selected,
        ),
    }
}

/// The query line: one row, no frame.
///
/// A bordered input box here would cost three rows to hold one, and those two
/// rows come off the list somebody is searching. The `/` says what the row is
/// as plainly as a heading would, and it is the key that opened it.
fn render_query(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let say = crate::i18n::screens(app.interface);
    let typing = app.focus == Focus::Query;
    let colour = if typing { Color::Cyan } else { Color::DarkGray };
    let room = usize::from(area.width).saturating_sub(4);
    // Empty and untouched, the row says what the key is for. That prompt is the
    // whole reason the row is always drawn — it is the only thing on the screen
    // that says searching is possible.
    let (text, style) = if app.query.is_empty() && !typing {
        (
            say.search_placeholder.to_owned(),
            Style::default().fg(Color::DarkGray),
        )
    } else {
        (
            truncate(app.query.trim_start(), room),
            Style::default()
                .fg(if typing { Color::White } else { Color::Gray })
                .add_modifier(Modifier::BOLD),
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" / ", Style::default().fg(colour)),
            Span::styled(text, style),
        ])),
        area,
    );

    if typing {
        // The terminal's own cursor rather than a drawn one, so it blinks like
        // every other place text is typed.
        let offset = u16::try_from(app.query.chars().count().min(room)).unwrap_or(u16::MAX);
        frame.set_cursor_position(Position {
            x: area.x.saturating_add(3).saturating_add(offset),
            y: area.y,
        });
    }
}

/// The prompts list: what was asked, most recent first.
///
/// The dashboard counted them and offered no way to see one. They are the other
/// half of what a session recorded — an observation says what was concluded, and
/// the prompt says what was asked to get there.
fn render_prompt_list(
    frame: &mut Frame<'_>,
    area: Rect,
    chrome: ListChrome<'_>,
    prompts: &[Prompt],
    selected: usize,
) {
    let ListChrome {
        language,
        title,
        position,
        focused,
    } = chrome;
    let items = if prompts.is_empty() {
        vec![ListItem::new(Line::styled(
            crate::i18n::screens(language).no_prompts,
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        prompts
            .iter()
            .map(|prompt| {
                // One line each: a prompt runs to paragraphs, and a list of
                // paragraphs is not a list. Enter puts the whole one on the
                // status line.
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<16}", truncate(&prompt.project, 15)),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(truncate(
                        prompt.content.trim().replace('\n', " ").trim(),
                        usize::from(area.width).saturating_sub(22),
                    )),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(list_panel(title, position, focused))
        .highlight_symbol("> ")
        .highlight_style(selected_style());
    let mut state = ListState::default();
    if !prompts.is_empty() {
        state.select(Some(clamp_index(selected, prompts.len())));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

/// The dashboard before anything has been saved.
///
/// No drawing here. The home screen carries the identity, and the same cat on
/// two screens one keypress apart reads as a placeholder rather than a mascot.
/// What is left is the sentence, which is the part that helps.
fn render_empty_dashboard(frame: &mut Frame<'_>, app: &App, area: Rect) {
    // Two languages on one screen, on purpose: the panel and the two sentences
    // under it are Leteo's own, and the line between them is Sardi's. They are
    // the same language unless somebody has said otherwise, and this is the
    // screen where saying otherwise has to show.
    let say = crate::i18n::screens(app.interface);
    let panel = panel(say.panel_dashboard);
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    let caption = [
        String::new(),
        crate::sardi::empty(app.voice_interface),
        say.empty_dashboard_what_happens.to_owned(),
        say.empty_dashboard_keys.to_owned(),
    ];

    let mut lines: Vec<Line<'_>> = Vec::new();
    for (index, text) in caption.iter().enumerate() {
        let style = if index == 1 {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::styled(text.clone(), style));
    }

    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .min(inner.height);
    let target = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(height) / 2,
        width: inner.width,
        height,
    };
    frame.render_widget(
        Paragraph::new(Text::from(lines)).alignment(Alignment::Center),
        target,
    );
}

/// The filter panel: a checkbox per project.
///
/// It used to be the project names joined by commas, which named them and let
/// somebody do nothing about it. With sixteen projects in one store the recent
/// list is sixteen projects deep, and the store could already narrow — the
/// screen just never asked it to.
///
/// Checkboxes rather than one choice, and the same `[✓]` the setup wizard uses:
/// asking about `leteo` and `engram` together is a question somebody has, and
/// one query answers it. No marks means every project, so clearing the filter is
/// unticking rather than a key nobody would guess.
fn render_filters(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let say = crate::i18n::screens(app.interface);
    let focused = app.focus == Focus::Filters;
    let items: Vec<ListItem<'_>> = if app.stats.projects.is_empty() {
        vec![ListItem::new(Line::styled(
            say.no_projects,
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.stats
            .projects
            .iter()
            .map(|project| {
                let marked = app.projects_filter.iter().any(|p| p == project);
                let style = if marked {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                ListItem::new(Line::styled(
                    format!("[{}] {project}", if marked { '\u{2713}' } else { ' ' }),
                    style,
                ))
            })
            .collect()
    };

    // The heading says how much is in force, because the marks scroll out of
    // sight in a panel this size and a filter nobody can see is a filter nobody
    // remembers setting.
    //
    // It counts the search as well as the projects, even though the search is
    // typed on its own row above rather than ticked in here. The panel selects
    // projects; the heading is about what the lists are narrowed by, and a
    // heading that said FILTERS with a search running would be wrong.
    let active = app.projects_filter.len() + usize::from(!app.query.trim().is_empty());
    let title = match active {
        0 => say.panel_filters.to_owned(),
        n => fill(say.panel_filters_count, "count", n),
    };
    let mut state = ListState::default();
    if focused && !app.stats.projects.is_empty() {
        state.select(Some(clamp_index(
            app.project_selected,
            app.stats.projects.len(),
        )));
    }
    let list = List::new(items)
        .block(focus_panel(&title, focused))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan));
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_observation_list(
    frame: &mut Frame<'_>,
    area: Rect,
    chrome: ListChrome<'_>,
    observations: &[Observation],
    selected: usize,
) {
    let ListChrome {
        language,
        title,
        position,
        focused,
    } = chrome;
    let items = if observations.is_empty() {
        vec![ListItem::new(Line::styled(
            crate::i18n::screens(language).no_observations,
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        observations
            .iter()
            .map(|observation| {
                let project = observation.project.as_deref().unwrap_or("global");
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("#{:<5}", observation.id),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        format!("{:<12}", truncate(&observation.kind, 11)),
                        Style::default().fg(Color::Magenta),
                    ),
                    Span::raw(format!("{}  ", truncate(&observation.title, 54))),
                    Span::styled(format!("[{project}]"), Style::default().fg(Color::DarkGray)),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(list_panel(title, position, focused))
        .highlight_symbol("> ")
        .highlight_style(selected_style());
    let mut state = ListState::default();
    if !observations.is_empty() {
        state.select(Some(clamp_index(selected, observations.len())));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

pub(super) fn render_detail(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let say = crate::i18n::screens(app.interface);
    let Some(observation) = &app.detail else {
        frame.render_widget(
            Paragraph::new(say.no_observation_selected).block(panel(say.panel_detail)),
            area,
        );
        return;
    };
    // The facts pane grows by a line per caveat rather than scrolling them out
    // of sight: "this was overturned" is the one thing on this page that
    // changes what somebody should do with what they are about to read.
    let facts = 8 + u16::try_from(app.detail_caveats.len()).unwrap_or(0);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(facts), Constraint::Min(1)])
        .split(area);
    let project = observation.project.as_deref().unwrap_or("global");
    let topic = observation.topic_key.as_deref().unwrap_or("-");
    let mut metadata = vec![
        field_line("ID", observation.id.to_string()),
        field_line(say.field_type, observation.kind.clone()),
        field_line(say.field_project, project.to_owned()),
        field_line(say.field_scope, observation.scope.clone()),
        field_line(say.field_session, observation.session_id.clone()),
        field_line(say.field_topic, topic.to_owned()),
    ];
    for caveat in &app.detail_caveats {
        // Yellow, and named by the verb rather than by a symbol: the reader has
        // to know which of the two it is without a legend.
        metadata.push(Line::from(vec![
            Span::styled(
                format!("{:<10}", caveat.verb.phrase()),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!("#{} ", caveat.other_id),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(truncate(&caveat.other_title, 52)),
        ]));
    }
    let metadata = Text::from(metadata);
    frame.render_widget(
        Paragraph::new(metadata).block(panel(&format!(" {} ", observation.title))),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(observation.content.as_str())
            .wrap(Wrap { trim: false })
            .scroll((app.detail_scroll, 0))
            .block(panel(say.panel_content)),
        rows[1],
    );
}

/// One session: what it was, and everything it recorded.
///
/// The dashboard's sessions list gives a row per session and no way past it,
/// which made the count on each row a fact nobody could act on. This is what
/// Enter opens, and it is laid out the way the detail page is: the facts at the
/// top, the substance filling the rest.
///
/// The observations are a list rather than prose because they are what somebody
/// came for — Enter on one opens it, so a session is a way into memories as well
/// as a record of them.
pub(super) fn render_session(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let say = crate::i18n::screens(app.interface);
    let Some((session, entries)) = &app.session else {
        frame.render_widget(
            Paragraph::new(say.no_session_selected).block(panel(say.panel_session)),
            area,
        );
        return;
    };
    let summary = session.summary.as_deref().unwrap_or(say.no_summary);
    // Four field rows, a blank, the summary heading, and room for the summary to
    // wrap over a couple of lines. Fixed rather than measured: a session with a
    // page-long summary would otherwise push its own observations off screen.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(3)])
        .split(area);
    let facts = Text::from(vec![
        field_line(say.field_project, session.project.clone()),
        field_line(say.field_started, session.started_at.clone()),
        field_line(
            say.field_ended,
            session
                .ended_at
                .clone()
                .unwrap_or_else(|| say.session_active.to_owned()),
        ),
        Line::from(""),
        Line::styled(say.field_summary, Style::default().fg(Color::Cyan)),
        Line::raw(summary.to_owned()),
    ]);
    frame.render_widget(
        Paragraph::new(facts)
            .wrap(Wrap { trim: false })
            .block(panel(&format!(" {} ", session.id))),
        rows[0],
    );
    let height = usize::from(rows[1].height.saturating_sub(2));
    app.list_height.set(height.max(1));
    render_observation_list(
        frame,
        rows[1],
        ListChrome {
            language: app.interface,
            title: &fill(say.panel_recorded, "count", app.session_entry_total),
            position: &position(
                app.interface,
                app.session_entry_offset,
                app.session_entry_selected,
                height,
                app.session_entry_total,
            ),
            focused: true,
        },
        entries,
        app.session_entry_selected,
    );
}

fn render_session_list(
    frame: &mut Frame<'_>,
    area: Rect,
    chrome: ListChrome<'_>,
    sessions: &[SessionSummary],
    selected: usize,
) {
    let ListChrome {
        language,
        title,
        position,
        focused,
    } = chrome;
    let items = if sessions.is_empty() {
        vec![ListItem::new(Line::styled(
            crate::i18n::screens(language).no_sessions,
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        sessions
            .iter()
            .map(|session| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<16}", truncate(&session.project, 15)),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(format!(
                        "{:<24} {:>4}",
                        truncate(&session.id, 23),
                        session.observation_count
                    )),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(list_panel(title, position, focused))
        .highlight_symbol("> ")
        .highlight_style(selected_style());
    let mut state = ListState::default();
    if !sessions.is_empty() {
        state.select(Some(clamp_index(selected, sessions.len())));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

pub(super) fn render_timeline(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let say = crate::i18n::screens(app.interface);
    let Some(timeline) = &app.timeline else {
        frame.render_widget(
            Paragraph::new(say.no_timeline_loaded).block(panel(say.panel_timeline)),
            area,
        );
        return;
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(1)])
        .split(area);
    let session = timeline
        .session_info
        .as_ref()
        .map_or(timeline.focus.session_id.as_str(), |session| {
            session.id.as_str()
        });
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(fill(say.timeline_session, "session", session)),
            Line::from({
                let text = fill(say.timeline_focus, "id", timeline.focus.id);
                let text = fill(&text, "title", &timeline.focus.title);
                fill(
                    &text,
                    "total",
                    timeline.before_total + timeline.after_total + 1,
                )
            }),
        ])
        .block(panel(say.panel_context)),
        rows[0],
    );

    let mut items = Vec::with_capacity(timeline.before.len() + 1 + timeline.after.len());
    for entry in &timeline.before {
        items.push(timeline_item(
            app.interface,
            entry.id,
            &entry.kind,
            &entry.title,
            &entry.created_at,
            false,
        ));
    }
    items.push(timeline_item(
        app.interface,
        timeline.focus.id,
        &timeline.focus.kind,
        &timeline.focus.title,
        &timeline.focus.created_at,
        true,
    ));
    for entry in &timeline.after {
        items.push(timeline_item(
            app.interface,
            entry.id,
            &entry.kind,
            &entry.title,
            &entry.created_at,
            false,
        ));
    }
    let list = List::new(items)
        .block(panel(say.panel_session_timeline))
        .highlight_symbol("> ")
        .highlight_style(selected_style());
    let len = timeline.before.len() + 1 + timeline.after.len();
    let mut state = ListState::default();
    state.select(Some(clamp_index(app.timeline_selected, len)));
    frame.render_stateful_widget(list, rows[1], &mut state);
}

fn timeline_item(
    language: crate::settings::Interface,
    id: i64,
    kind: &str,
    title: &str,
    created_at: &str,
    focus: bool,
) -> ListItem<'static> {
    let marker = if focus {
        crate::i18n::screens(language).timeline_focus_marker
    } else {
        "     "
    };
    ListItem::new(Line::from(vec![
        Span::styled(
            format!("{marker} "),
            Style::default()
                .fg(if focus {
                    Color::Yellow
                } else {
                    Color::DarkGray
                })
                .add_modifier(if focus {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::styled(
            format!("#{id:<5} {kind:<12}"),
            Style::default().fg(Color::Magenta),
        ),
        Span::raw(format!("{}  ", truncate(title, 48))),
        Span::styled(
            truncate(created_at, 19),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
}
