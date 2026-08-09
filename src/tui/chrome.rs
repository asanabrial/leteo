use super::*;

/// The confirmation window: what is about to go, and what goes with it.
///
/// Over the middle of the screen, and the screen behind it is cleared where it
/// sits so nothing shows through — a half-legible list under a warning reads as
/// a rendering fault and takes attention away from the one thing being asked.
///
/// Red, and the destructive answer named rather than assumed. `y` is the only
/// key that goes through: a window that appears under somebody's hands catches
/// whatever they were already pressing, and Enter is the likeliest of those.
fn render_confirmation(
    frame: &mut Frame<'_>,
    language: crate::settings::Interface,
    pending: &PendingAction,
    area: Rect,
) {
    let say = crate::i18n::screens(language);
    let mut lines = vec![
        Line::styled(
            pending.heading.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Line::raw(""),
    ];
    for (index, row) in pending.detail.iter().enumerate() {
        // The last line is the warning about what cannot be taken back.
        let style = if index + 1 == pending.detail.len() {
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::styled(row.clone(), style));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        say.keys_confirm_window,
        Style::default().fg(Color::DarkGray),
    ));

    let width = entry_width(&lines)
        .saturating_add(6)
        .min(area.width.saturating_sub(2))
        .max(1);
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2)
        .min(area.height)
        .max(1);
    let window = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, window);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Red)),
            ),
        window,
    );
}

pub(super) fn render(frame: &mut Frame<'_>, app: &App) {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(frame.area());

    render_header(frame, app, areas[0]);
    match app.page {
        Page::Home => render_home(frame, app, areas[1]),
        Page::Dashboard => render_dashboard(frame, app, areas[1]),
        Page::Detail => render_detail(frame, app, areas[1]),
        Page::Session => render_session(frame, app, areas[1]),
        Page::Timeline => render_timeline(frame, app, areas[1]),
        Page::Setup | Page::Options => render_setup(frame, app, areas[1]),
        Page::Cloud => render_cloud(frame, app, areas[1]),
        Page::Help => render_help(frame, app, areas[1]),
    }
    render_footer(frame, app, areas[2]);
    // Last, and over everything: a window nothing else can be read around.
    if let Some(pending) = &app.pending {
        render_confirmation(frame, app.interface, pending, frame.area());
    }
}

/// The setup page: the same wizard `leteo setup` runs.
///
/// It is not reimplemented here. The wizard returns lines tagged with a role,
/// and this maps roles onto ratatui styles the way the crossterm driver maps
/// them onto terminal attributes — one flow, two painters, no second set of
/// questions to keep in step with the first.
fn render_setup(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let say = crate::i18n::screens(app.interface);
    // The same painter serves both pages the wizard is behind, and the border
    // is the only thing that says which one somebody opened.
    let block = panel(if app.page == Page::Options {
        say.panel_options
    } else {
        say.panel_setup
    });
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(wizard) = app.wizard.as_ref() else {
        return;
    };
    // One row for the margin the flow is drawn at, so the count handed to the
    // wizard is the number of rows that will actually be painted rather than
    // the size of the panel around them.
    let available = usize::from(inner.height.saturating_sub(1));
    let lines: Vec<Line<'_>> = wizard
        .render_within(available)
        .into_iter()
        .map(|row| Line::styled(row.text, wizard_style(row.role)))
        .collect();
    // Top-aligned, like `leteo setup`. Centred, the questions would sit halfway
    // down the panel here and at the top there — the same flow reading as two
    // different screens, which is the thing sharing it was meant to avoid.
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .min(inner.height.saturating_sub(1));
    let target = Rect {
        x: inner.x + 2,
        y: inner.y + 1,
        width: inner.width.saturating_sub(2),
        height,
    };
    frame.render_widget(Paragraph::new(Text::from(lines)), target);
}

/// What each wizard role looks like here.
///
/// The same assignment as [`crate::wizard`]'s own palette, in the vocabulary of
/// a different renderer. Keeping them apart is the price of the wizard not
/// depending on ratatui, which is what lets `leteo setup` run without one.
fn wizard_style(role: crate::setup::wizard::Role) -> Style {
    use crate::setup::wizard::Role;
    match role {
        Role::Brand => Style::default()
            .fg(Color::Rgb(0x94, 0xe2, 0xd5))
            .add_modifier(Modifier::BOLD),
        Role::Heading => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        Role::Detail | Role::Hint => Style::default().fg(Color::DarkGray),
        Role::Choice => Style::default().fg(Color::White),
        Role::Focused => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    }
}
fn render_cloud(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let say = crate::i18n::screens(app.interface);
    // What a count reads as when nobody could ask for it. Deliberately not `0`
    // and not "none": those are what a healthy idle queue says, and the whole
    // point is that the two must not look alike.
    let unreadable = say.cloud_unknown;
    let cloud = &app.cloud;
    let mut lines = vec![
        Line::from(vec![
            Span::styled(say.cloud_server, Style::default().fg(Color::DarkGray)),
            Span::raw(if cloud.server.is_empty() {
                say.cloud_not_configured.to_owned()
            } else {
                cloud.server.clone()
            }),
        ]),
        Line::from(vec![
            Span::styled(say.cloud_background, Style::default().fg(Color::DarkGray)),
            Span::styled(
                if cloud.enabled {
                    say.cloud_enabled
                } else {
                    say.cloud_disabled
                },
                Style::default().fg(if cloud.enabled {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled(say.cloud_replicating, Style::default().fg(Color::DarkGray)),
            Span::raw(if cloud.projects.is_empty() {
                say.cloud_none.to_owned()
            } else {
                cloud.projects.join(", ")
            }),
        ]),
        Line::from(vec![
            Span::styled(say.cloud_enrolled, Style::default().fg(Color::DarkGray)),
            Span::raw(match &cloud.unreadable {
                Some(_) => unreadable.to_owned(),
                None if cloud.enrolled.is_empty() => say.cloud_none.to_owned(),
                None => cloud.enrolled.join(", "),
            }),
        ]),
        Line::from(vec![
            Span::styled(say.cloud_queued, Style::default().fg(Color::DarkGray)),
            Span::raw(match &cloud.unreadable {
                Some(_) => unreadable.to_owned(),
                None => fill(say.cloud_mutations, "count", cloud.pending_mutations),
            }),
        ]),
        Line::from(vec![
            Span::styled(say.cloud_deferred, Style::default().fg(Color::DarkGray)),
            Span::raw(match &cloud.unreadable {
                Some(_) => unreadable.to_owned(),
                None => fill(
                    &fill(say.cloud_deferred_dead, "deferred", cloud.deferred),
                    "dead",
                    cloud.dead,
                ),
            }),
        ]),
    ];
    // What replication is doing, beside how much is waiting.
    //
    // The counts above are the same four numbers whether the next cycle is a
    // minute away or sync has been refusing for three days. This line is the
    // difference, and it is why somebody opened this page.
    if let Some(state) = &cloud.state {
        let mut parts = vec![state.lifecycle.clone()];
        if state.consecutive_failures > 0 {
            parts.push(fill(
                say.cloud_failures,
                "count",
                state.consecutive_failures,
            ));
        }
        if let Some(until) = &state.backoff_until {
            parts.push(fill(say.cloud_backoff, "until", until));
        }
        lines.push(Line::from(vec![
            Span::styled(say.cloud_state, Style::default().fg(Color::DarkGray)),
            Span::styled(
                parts.join(", "),
                Style::default().fg(if state.consecutive_failures > 0 {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
        ]));
        // The message the server or the wire actually gave, in full and in red.
        // A lifecycle of "backoff" says something is wrong; only this says what,
        // and it is the one line that turns the page from a symptom into a fix.
        if let Some(reason) = state
            .last_error
            .as_deref()
            .or(state.reason_message.as_deref())
        {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                reason.to_owned(),
                Style::default().fg(Color::Red),
            )));
        }
    }
    // In red, and with the reason. A queue nobody could read is a worse state
    // than an empty one, so it must not be the quieter of the two on screen.
    if let Some(reason) = &cloud.unreadable {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            fill(say.cloud_unreadable, "reason", reason),
            Style::default().fg(Color::Red),
        )));
    }
    if !cloud.configured {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            say.cloud_configure_hint,
            Style::default().fg(Color::DarkGray),
        )));
    }
    let widget = Paragraph::new(Text::from(lines))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(say.panel_cloud),
        );
    frame.render_widget(widget, area);
}

fn render_header(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " LETEO ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            app.page.title(app.interface),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        // Whose store this is, said once where it is always in view.
        Span::styled(
            crate::sardi::watching(
                app.voice_interface,
                app.stats.total_observations,
                app.stats.projects.len(),
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(header, area);
}

/// The landing screen: the drawing and [`MENU`].
///
/// No frame around it. Every other page is a bordered panel because it holds a
/// list of somebody's data and the title says which; this one is a cat and a
/// menu, and a box around that is a second border directly under the header's
/// for no gain. It is also two rows, and two rows here is two rows of drawing.
///
/// Two arrangements, chosen by the room available rather than by preference.
/// Side by side the pair costs only as many rows as the drawing itself; stacked
/// it costs the drawing plus everything under it, a dozen rows more, which kept
/// the cat off any window shorter than about fifty.
///
/// What appears is graded by how much room there is, and the grading is by
/// worth rather than by size:
///
/// - The entries are why the screen exists and are always drawn.
/// - The wordmark appears only when the drawing does not — the cat *is* the
///   mark, so printing both says the same thing twice, and those three rows are
///   the ones that decide whether the cat fits at all.
/// - The heading and the version are the first to go: pleasant beside a
///   twenty-nine row drawing, not worth a row when rows are contested.
///
/// What the store holds is not repeated here at all — the header carries it on
/// every page.
fn render_home(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let art = crate::sardi::CAT_LARGE;
    let art_width = crate::sardi::art_width(art);
    let art_height = crate::sardi::art_height(art);

    let entries = menu_entries(app);
    let entry_height = u16::try_from(entries.len()).unwrap_or(u16::MAX);

    // Beside the drawing there is room to spare, so the menu gets a heading
    // above it and the version below. Neither is load-bearing: both are dropped
    // in the stacked arrangement, where every row is contested.
    let mut menu = vec![
        Line::styled("  ACTIONS", Style::default().fg(Color::DarkGray)),
        Line::raw(""),
    ];
    menu.extend(entries.iter().cloned());
    menu.push(Line::raw(""));
    menu.push(Line::styled(
        format!("  leteo {}", env!("CARGO_PKG_VERSION")),
        Style::default().fg(Color::DarkGray),
    ));
    let menu_width = entry_width(&menu);
    let menu_height = u16::try_from(menu.len()).unwrap_or(u16::MAX);

    // Wide enough that the cat and the menu read as two things rather than as
    // one block with a seam down it.
    const GAP: u16 = 10;
    let paired_width = art_width + GAP + menu_width;
    if area.width >= paired_width && area.height >= art_height {
        let left = area.x + (area.width - paired_width) / 2;
        let top = area.y + (area.height - art_height) / 2;
        frame.render_widget(
            Paragraph::new(Text::from(banded(art))),
            Rect {
                x: left,
                y: top,
                width: art_width,
                height: art_height,
            },
        );
        // Centred against the drawing rather than against the screen, so the
        // two sit on one axis whatever the window does.
        frame.render_widget(
            Paragraph::new(Text::from(menu)),
            Rect {
                x: left + art_width + GAP,
                y: top + art_height.saturating_sub(menu_height) / 2,
                width: menu_width,
                height: menu_height.min(area.height),
            },
        );
        return;
    }

    // Stacked, and without the drawing: what would not fit beside the menu will
    // not fit above it either.
    //
    // Down to the entries and the wordmark. The entries are why the screen
    // exists, and the wordmark is the only thing naming it once the cat is
    // gone — so the heading and the version are what go, not the mark.
    let banner = crate::sardi::banner(entry_width(&entries), "LETEO");
    let banner_height = u16::try_from(banner.len()).unwrap_or(0);
    let mut lines: Vec<Line<'_>> = Vec::new();
    if area.height >= banner_height + 1 + entry_height {
        lines.extend(
            banner
                .into_iter()
                .map(|row| Line::styled(row, Style::default().fg(Color::Cyan))),
        );
        lines.push(Line::raw(""));
    }
    lines.extend(entries);

    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .min(area.height);
    frame.render_widget(
        Paragraph::new(Text::from(lines)).alignment(Alignment::Center),
        Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(height) / 2,
            width: area.width,
            height,
        },
    );
}

/// How wide a block of rendered lines is.
fn entry_width(lines: &[Line<'static>]) -> u16 {
    lines
        .iter()
        .map(|line| u16::try_from(line.width()).unwrap_or(u16::MAX))
        .max()
        .unwrap_or(0)
}

/// One row per [`MENU`] entry, with the cursor on the selected one.
///
/// Every entry padded to one width so the cursor sits in a column rather than
/// stepping in and out with the length of each label.
fn menu_entries(app: &App) -> Vec<Line<'static>> {
    let say = crate::i18n::screens(app.interface);
    let label_width = MENU
        .iter()
        .map(|(label, _)| label(say).chars().count())
        .max()
        .unwrap_or(0);

    MENU.iter()
        .enumerate()
        .map(|(index, (label, _))| {
            let label = label(say);
            let chosen = index == app.home_selected;
            // The cursor is part of the line rather than a highlight: a reversed row
            // reads as a bar across the screen, and beside a drawing it reads as a
            // fault in it.
            let marker = if chosen { '\u{25b8}' } else { ' ' };
            let style = if chosen {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                // Dimmer than the cursor line by a whole step, so which entry is
                // selected is obvious at a glance rather than on inspection.
                Style::default().fg(Color::Gray)
            };
            Line::styled(format!("{marker} {label:<label_width$}"), style)
        })
        .collect()
}

/// A drawing's rows, coloured top to bottom.
pub(super) fn banded(art: &'static [&'static str]) -> Vec<Line<'static>> {
    art.iter()
        .enumerate()
        .map(|(index, row)| {
            let (red, green, blue) = crate::sardi::band(index, art.len());
            Line::styled(*row, Style::default().fg(Color::Rgb(red, green, blue)))
        })
        .collect()
}

/// Where the cursor is in the whole list, for the corner of its frame.
///
/// The row rather than the span of rows read: the store is read four hundred at
/// a time and the panel shows perhaps thirty, so a corner reading `1–400 of
/// 3313` describes the fetch and contradicts what is on screen. `312 of 3313`
/// is the one thing somebody wants from that corner — how far in they are.
///
/// Empty when the whole list is on screen, where the answer is visible already.
pub(super) fn position(
    language: crate::settings::Interface,
    offset: usize,
    selected: usize,
    height: usize,
    total: i64,
) -> String {
    let total = usize::try_from(total).unwrap_or(0);
    if total <= height {
        return String::new();
    }
    let say = crate::i18n::screens(language);
    fill(
        &fill(say.list_position, "position", offset + selected + 1),
        "total",
        total,
    )
}

/// A panel whose border says whether the arrow keys are pointed at it.
pub(super) fn focus_panel(title: &str, focused: bool) -> Block<'_> {
    let colour = if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(colour))
        // Written the way `panel` writes it — no padding — so a focused panel
        // differs from its neighbours in colour alone rather than also shifting
        // its heading along by a character.
        .title(Span::styled(title.to_owned(), Style::default().fg(colour)))
}

/// The frame around a list: the heading at the top, where you are at the foot.
///
/// The position goes in the bottom-right corner rather than into the heading.
/// A heading already carrying what the list is, what it matches and which
/// projects it is limited to has no room left for `1–100 of 3312`, and on a
/// narrow terminal it is the position that would be clipped away — which is the
/// part that changes as somebody moves.
pub(super) fn list_panel<'a>(title: &'a str, position: &'a str, focused: bool) -> Block<'a> {
    let block = focus_panel(title, focused);
    if position.is_empty() {
        return block;
    }
    block.title_bottom(
        Line::styled(position.to_owned(), Style::default().fg(Color::DarkGray)).right_aligned(),
    )
}

pub(super) fn field_line(label: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<9}"), Style::default().fg(Color::DarkGray)),
        Span::raw(value),
    ])
}

fn render_help(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let say = crate::i18n::screens(app.interface);
    // A section heading is a line with no leading space, which is what the
    // catalogue's own shape already says. Parsing it here rather than storing
    // forty fields keeps the columns visible to whoever translates them.
    let help = Text::from(
        say.help_body
            .lines()
            .map(|line| {
                if line.is_empty() || line.starts_with(' ') {
                    Line::from(line)
                } else {
                    Line::styled(
                        line,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                }
            })
            .collect::<Vec<_>>(),
    );
    frame.render_widget(
        Paragraph::new(help)
            .wrap(Wrap { trim: false })
            .block(panel(say.panel_help)),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let say = crate::i18n::screens(app.interface);
    let shortcuts = match app.page {
        _ if app.pending.is_some() => say.keys_confirm,
        Page::Home => say.keys_home,
        Page::Dashboard if app.focus == Focus::Query => say.keys_query,
        Page::Dashboard if app.focus == Focus::Filters => say.keys_filters,
        // What Esc does here changes once there is a search to drop, and that
        // is the one somebody would otherwise get wrong — so with a query
        // running the footer says so rather than promising to go back.
        Page::Dashboard if !app.query.trim().is_empty() => say.keys_dashboard_searching,
        Page::Dashboard if app.list == ListKind::Sessions => say.keys_dashboard_sessions,
        Page::Dashboard if app.list == ListKind::Prompts => say.keys_dashboard_prompts,
        Page::Dashboard => say.keys_dashboard,
        Page::Detail => say.keys_detail,
        Page::Session => say.keys_session,
        Page::Timeline => say.keys_timeline,
        Page::Setup => say.keys_setup,
        // Not the setup keys: nothing here is ticked, there is nothing to
        // continue to, and Esc steps back a screen rather than abandoning the
        // page.
        Page::Options => say.keys_options,
        Page::Cloud => say.keys_cloud,
        Page::Help => say.keys_help,
    };
    // While a confirmation is up, the footer says what answers it and nothing
    // else. The rest of the keys do not work, and listing them would be an
    // invitation to press one.
    if app.pending.is_some() {
        frame.render_widget(
            Paragraph::new(Line::styled(
                say.keys_confirm_footer,
                Style::default().fg(Color::DarkGray),
            )),
            area,
        );
        return;
    }
    let second_line = app.status.as_ref().map_or_else(
        || {
            Line::styled(
                truncate(&app.database_path, usize::from(area.width)),
                Style::default().fg(Color::DarkGray),
            )
        },
        |status| {
            Line::styled(
                truncate(&status.text, usize::from(area.width)),
                Style::default().fg(if status.is_error {
                    Color::Red
                } else {
                    Color::Green
                }),
            )
        },
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(shortcuts, Style::default().fg(Color::DarkGray)),
            second_line,
        ]),
        area,
    );
}

pub(super) fn panel(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(title.to_owned())
}

pub(super) fn selected_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    if max_chars <= 3 {
        return value.chars().take(max_chars).collect();
    }
    let mut output: String = value.chars().take(max_chars - 3).collect();
    output.push_str("...");
    output
}
