use anyhow::Result;

use super::screen::palette;
use super::{Offer, Outcome, Wizard};

pub fn run_interactive(offer: Offer) -> Result<Outcome> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind};
    use crossterm::style::{Attribute, SetAttribute, SetForegroundColor};
    use crossterm::terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode};
    use crossterm::{cursor, queue};
    use std::io::Write;

    let mut wizard = Wizard::new(offer);
    let mut stdout = std::io::stdout();
    enable_raw_mode()?;
    // Raw mode has to come off however this ends, including on an error, or the
    // shell is left unusable.
    let result = (|| -> Result<()> {
        let mut painted = 0_u16;
        let mut dirty = true;
        loop {
            if dirty {
                // Asked for on every frame rather than once, because a terminal
                // can be resized under a flow that is waiting for a key.
                //
                // One row short of the window on purpose: painting exactly as
                // many lines as the terminal is tall scrolls it by one, and the
                // `MoveUp` that redraws in place would then be aimed a row off
                // and walk the screen upwards on every keystroke. A size the
                // terminal will not report means no window at all, which is
                // what this did before it had one.
                let height = crossterm::terminal::size()
                    .map_or(usize::MAX, |(_, rows)| usize::from(rows).saturating_sub(1));
                let lines = wizard.render_within(height);
                // Redraw in place rather than scrolling the screen away.
                //
                // Through crossterm rather than by writing escape sequences by
                // hand: the Windows console does not interpret them unless
                // virtual terminal processing happens to be enabled, and prints
                // them as rubbish when it is not. crossterm picks the console
                // API or the escape sequence to suit the platform it is on.
                if painted > 0 {
                    queue!(stdout, cursor::MoveUp(painted))?;
                }
                for line in &lines {
                    queue!(stdout, Clear(ClearType::CurrentLine))?;
                    // Colour through crossterm for the same reason as the cursor
                    // moves above: on Windows it sets the console attribute
                    // rather than emitting an escape the console would print.
                    let (colour, bold) = palette(line.role);
                    queue!(stdout, SetForegroundColor(colour))?;
                    if bold {
                        queue!(stdout, SetAttribute(Attribute::Bold))?;
                    }
                    write!(stdout, "{}", line.text)?;
                    queue!(stdout, SetAttribute(Attribute::Reset))?;
                    write!(stdout, "\r\n")?;
                }
                queue!(stdout, Clear(ClearType::FromCursorDown))?;
                painted = u16::try_from(lines.len()).unwrap_or(u16::MAX);
                stdout.flush()?;
                dirty = false;
            }
            if wizard.is_finished() {
                return Ok(());
            }

            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            dirty = match key.code {
                KeyCode::Up | KeyCode::Char('k') => wizard.up(),
                KeyCode::Down | KeyCode::Char('j') => wizard.down(),
                KeyCode::Char(' ') => wizard.toggle(),
                KeyCode::Enter => wizard.advance(),
                KeyCode::Backspace => wizard.back(),
                KeyCode::Esc | KeyCode::Char('q') => wizard.cancel(),
                _ => false,
            };
        }
    })();
    disable_raw_mode()?;
    result?;

    writeln!(stdout)?;
    let outcome = wizard.apply(&mut stdout)?;
    Ok(outcome)
}
