#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Brand,
    Heading,
    Detail,
    Choice,
    Focused,
    Hint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub role: Role,
}

impl Row {
    pub(super) fn new(role: Role, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            role,
        }
    }

    pub(super) fn blank() -> Self {
        Self::new(Role::Detail, "")
    }
}

const CURSOR: char = '\u{25b8}';

const TICK: char = '\u{2713}';
const DOT: char = '\u{25cf}';

pub(super) const BRAND: &str = "L E T E O";

pub(super) fn checkbox(focused: bool, ticked: bool, label: &str) -> Row {
    Row::new(
        if focused { Role::Focused } else { Role::Choice },
        format!(
            "{} [{}] {label}",
            if focused { CURSOR } else { ' ' },
            if ticked { TICK } else { ' ' }
        ),
    )
}

pub(super) fn radio(focused: bool, picked: bool, label: &str) -> Row {
    Row::new(
        if focused { Role::Focused } else { Role::Choice },
        format!(
            "{} ({}) {label}",
            if focused { CURSOR } else { ' ' },
            if picked { DOT } else { ' ' }
        ),
    )
}

pub(super) fn entry(focused: bool, label: &str, width: usize, value: &str) -> Row {
    let padding = " ".repeat(width.saturating_sub(label.chars().count()));
    Row::new(
        if focused { Role::Focused } else { Role::Choice },
        format!(
            "{} {label}{padding}    {value}",
            if focused { CURSOR } else { ' ' }
        ),
    )
}

const MORE: &str = "  \u{22ef}";

pub(super) fn windowed(rows: Vec<Row>, height: usize) -> Vec<Row> {
    if rows.len() <= height {
        return rows;
    }
    if height == 0 {
        return Vec::new();
    }
    let is_choice = |row: &Row| matches!(row.role, Role::Choice | Role::Focused);
    let (Some(first), Some(last)) = (
        rows.iter().position(is_choice),
        rows.iter().rposition(is_choice),
    ) else {
        return rows.into_iter().take(height).collect();
    };

    let mut head = first;
    let mut tail = rows.len() - last - 1;
    while head + tail + 1 > height {
        if tail > 0 {
            tail -= 1;
        } else if head > 0 {
            head -= 1;
        } else {
            break;
        }
    }
    let budget = height.saturating_sub(head + tail).max(1);

    let body = &rows[first..=last];
    let focused = body.iter().position(|row| row.role == Role::Focused);
    let start = match focused {
        Some(_) if body.len() <= budget => 0,
        Some(at) => at
            .saturating_sub(budget / 2)
            .min(body.len().saturating_sub(budget)),
        None => 0,
    };
    let end = (start + budget).min(body.len());

    let mut window: Vec<Row> = body[start..end].to_vec();
    if budget >= 3 {
        if start > 0 {
            window[0] = Row::new(Role::Hint, MORE);
        }
        if end < body.len() {
            let bottom = window.len() - 1;
            window[bottom] = Row::new(Role::Hint, MORE);
        }
    }

    let mut shown: Vec<Row> = rows[first - head..first].to_vec();
    shown.extend(window);
    shown.extend_from_slice(&rows[rows.len() - tail..]);
    shown
}

pub(super) fn palette(role: Role) -> (crossterm::style::Color, bool) {
    use crossterm::style::Color;
    match role {
        Role::Brand => (
            Color::Rgb {
                r: 0x94,
                g: 0xe2,
                b: 0xd5,
            },
            true,
        ),
        Role::Heading => (Color::Reset, true),
        Role::Detail => (Color::DarkGrey, false),
        Role::Choice => (Color::Reset, false),
        Role::Focused => (Color::Cyan, true),
        Role::Hint => (Color::DarkGrey, false),
    }
}
