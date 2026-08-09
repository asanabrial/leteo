//! What a line of the wizard is, and what the pieces on it look like.
//!
//! The state machine says what each line *means* and this says how it is drawn.
//! Keeping the two apart is what lets a test assert on the words: colour codes
//! in the returned strings would put terminal escapes inside every assertion,
//! and each one would then be checking the palette as well as the text.

/// What a rendered line is for.
///
/// The state machine says what each line means and the driver decides what that
/// looks like. Colour codes in the returned strings would put terminal escapes
/// inside the thing the tests assert on, and every assertion would then be
/// checking the palette as well as the words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The wordmark.
    Brand,
    /// A question, or the heading of a section.
    Heading,
    /// Supporting detail under a heading.
    Detail,
    /// A choice the cursor is not on.
    Choice,
    /// The choice under the cursor.
    Focused,
    /// The key legend, and anything else the eye should be able to skip.
    Hint,
}

/// One line of the wizard's screen.
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

/// The cursor, shaped like the one in the terminal UI so the two read as one
/// program rather than two.
const CURSOR: char = '\u{25b8}';

/// A ticked box, and a picked radio.
const TICK: char = '\u{2713}';
const DOT: char = '\u{25cf}';

/// The wordmark, spaced so it reads as a mark rather than as a word.
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

/// A row of the options index: a setting, and what it is set to.
///
/// No box and no dot in front of it. Those say "one of these is on", which is
/// the question *behind* the row; the row itself is a door, and the only thing
/// it has to say is where it leads and what is through it.
pub(super) fn entry(focused: bool, label: &str, width: usize, value: &str) -> Row {
    // Padded by characters rather than by bytes, because these labels are
    // translated and an accent is two bytes and one column.
    let padding = " ".repeat(width.saturating_sub(label.chars().count()));
    Row::new(
        if focused { Role::Focused } else { Role::Choice },
        format!(
            "{} {label}{padding}    {value}",
            if focused { CURSOR } else { ' ' }
        ),
    )
}

/// What stands in for the choices a window is too short to show.
///
/// Aligned under the cursor column, and the same character at both ends. It
/// carries no words on purpose: it is the one thing on these screens that would
/// otherwise need translating into twelve languages to say "there is more".
const MORE: &str = "  \u{22ef}";

/// The part of a screen that fits in `height` rows, with the cursor in it.
///
/// # Why the whole screen is not simply cut off
///
/// It was, and the tail is where the interesting rows are. A question with
/// thirteen answers under it runs to twenty-one rows, so on a short terminal
/// the last few languages and the key legend were both simply not drawn — and
/// the cursor could be moved onto a row nobody could see, which reads as the
/// keys having stopped working.
///
/// So the three parts of a screen are treated differently, by what they are
/// worth when there is not room for all of them:
///
/// - The **question** is kept. A list of answers to a question that has scrolled
///   away is a list of words.
/// - The **legend** is kept, for the same reason it exists at all: nobody can
///   guess that space ticks and backspace goes back.
/// - The **choices** are what scrolls, always including the one under the
///   cursor.
///
/// When even that does not fit, the parts give way from the outside in — the
/// hints above the legend first, then the wordmark — because one visible choice
/// is worth more than either.
///
/// The window is derived from where the cursor is rather than remembered, so
/// there is no scroll position to be carried between two drivers and get out of
/// step with the state machine. The cost is that the cursor sits in the middle
/// of a long list rather than at the edge it walked in from, which is what
/// `less` does and nobody minds.
pub(super) fn windowed(rows: Vec<Row>, height: usize) -> Vec<Row> {
    if rows.len() <= height {
        return rows;
    }
    // No room at all, which is a real answer rather than a reason to fall back
    // on the unwindowed screen: the alternative reading — "0 means no limit" —
    // would hand a caller with a two-row panel twenty-one rows to paint.
    if height == 0 {
        return Vec::new();
    }
    let is_choice = |row: &Row| matches!(row.role, Role::Choice | Role::Focused);
    let (Some(first), Some(last)) = (
        rows.iter().position(is_choice),
        rows.iter().rposition(is_choice),
    ) else {
        // Nothing to scroll — the last screen of the flow has no choices on it
        // at all. What fits is what is drawn, from the top.
        return rows.into_iter().take(height).collect();
    };

    // Both ends keep their *last* rows, which is what keeps the question and
    // the legend: the wordmark is at the top of the head and the hints are at
    // the top of the tail, and those are the two the screen can do without.
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
    // Said with a row rather than by making room for one, so the marker never
    // costs the screen a choice it had space for. Not at all on a window of two
    // or fewer, where the row it would replace could be the cursor's.
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

/// What each role looks like: a colour, and whether it is bold.
///
/// One place, so the wizard and the terminal UI can be compared side by side
/// rather than hunted for. The cyan is the one the UI already uses for its
/// header and its counters.
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
