mod voices;

use crate::settings::Interface;

pub const NAME: &str = "Sardi";

/// Every line is the same width, and blank cells are U+2800 — the Braille
/// pattern with no dots, not spaces, because a space is a different width in
/// some terminals and the drawing would shear. The source used U+2804, a *lit*
/// pattern, as its empty value; that is what image converters tend to do, and
/// why pasted art arrives with a grey wash behind it. Swapping it was safe only
/// because U+2800 appeared nowhere in the source. With both present the lit one
/// is carrying meaning and has to stay.
///
/// It is 45 by 29 characters, 90 by 116 dots, and that height is the whole
/// constraint on how it can be shown. With the caption under it the block wants
/// a terminal around forty rows deep, so [`crate::tui`] drops it when there is
/// not room rather than overwriting the words beside it.
pub const CAT_LARGE: &[&str] = &[
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⣤⣄⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣠⣤⣄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⡼⠟⠀⠀⢇⡀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⠿⠁⠀⠙⠿⣦⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣼⠟⢁⠀⠀⠀⢸⡇⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢸⠀⠀⠀⠰⣄⡈⠳⣦⡀⠀⠀⠀⠀⠀⢀⣰⡾⠋⣠⠎⠁⠀⠀⢸⡇⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢸⠀⠀⠀⠀⠀⠙⡄⠈⢻⣿⣿⠿⣿⣿⡿⣿⣇⠰⢿⣤⠀⠀⠀⢸⡇⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠸⣀⠀⠀⠀⢉⣩⠟⠂⠀⠙⢿⣤⠘⣿⣧⠘⣿⡀⠀⠈⠉⠢⣀⢸⡇⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣿⠀⠀⢠⠃⠀⠀⠀⢀⣀⠘⣿⡄⢻⣿⠀⣿⠃⣤⠤⠀⠀⠀⠻⣧⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⣇⠀⠃⠀⠀⠀⠀⠉⠉⠀⠈⠇⠈⠏⠀⠇⠀⢀⣤⣤⣄⠀⠀⠉⣇⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡿⠃⠀⠀⠀⢠⣴⠟⣛⠳⣤⠀⠀⠀⠀⠀⢀⠞⣶⡀⢻⠛⢶⣄⣸⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣀⡇⠀⢀⣤⡶⢻⠁⢰⣿⡆⢸⡇⠀⠀⠀⠀⢸⡻⠿⠃⡘⠀⠀⠉⢻⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣴⣾⢿⣿⣷⣶⣿⠏⠁⠀⠱⢄⣉⡠⠎⠇⠀⢀⣀⣀⡀⠉⠒⠊⠀⠀⠾⣿⡎⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⢀⣴⣿⠏⠁⢸⣿⡿⣯⣀⣴⣶⡦⠀⢀⣀⣀⠀⠀⠀⠙⢿⠟⠁⠀⠀⠐⠒⠒⢔⣒⡉⠉⠉⠑⠂⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⣾⡿⠇⠀⣀⣿⣿⠁⠉⢿⡉⠁⠀⠀⢀⣀⠶⠀⠀⢀⣀⠞⢦⣀⣀⡀⠈⠱⢦⣎⠁⠈⠉⠶⡀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⣼⣿⠁⠀⢠⣿⣿⠀⠀⣤⠈⠛⣷⣦⡖⠁⠀⠀⠀⠉⠉⠁⠀⠀⠈⠉⠀⣤⣾⣿⠀⠉⢆⠀⠀⠈⠀⠀⠀",
    "⠀⠀⠀⠀⠀⣤⣿⣿⡀⠀⢸⣿⡇⠀⠀⣿⣧⡀⠈⠙⠻⢿⣶⣶⣤⣤⣤⣤⣤⣤⣶⣾⠿⠛⠁⢸⣤⠀⠀⠛⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⣿⣿⣿⣇⠀⠈⢿⡇⠀⠀⠉⠻⣷⣤⡀⠀⠈⠉⠙⠛⠛⠛⠛⠛⠛⠉⠉⠀⠀⢀⣾⣿⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⣿⡇⢹⣿⣶⡀⠀⠹⠀⠀⣀⠀⠈⠉⠛⠳⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠰⠋⠁⣿⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⣿⣧⠀⠈⠛⣷⣦⡄⠀⠀⣿⡄⢰⣶⣦⡄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢠⣶⣾⠛⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠉⣿⣦⣄⠀⠈⠉⠉⠀⠸⣿⣷⠀⠉⠛⠛⠓⠀⢢⠀⠀⠀⠀⢴⡏⠠⠔⠚⠋⢩⡜⠀⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⢀⣀⣾⡿⢿⣿⣶⣶⣶⠿⠿⠿⢿⣇⠀⣰⣶⣆⠀⠈⣿⣀⣀⣰⡏⠀⠀⠰⢶⣶⣿⣷⣶⣀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⣤⡶⠿⠿⣿⣿⣿⣦⣤⣿⡏⠀⠀⠀⡄⠀⢻⣿⠋⠁⠀⠀⠀⠉⠻⣿⡟⠀⠀⠀⠀⠀⠈⣿⣿⣿⣿⣿⣷⣶⡦⠤⣄⡀",
    "⢸⣿⢱⣶⣦⣭⣭⣙⣛⠛⠿⠷⢶⣤⣤⣧⣤⣾⣿⡀⠀⢠⠀⠀⡄⢀⣿⣇⠀⢰⠀⢰⣀⣠⣿⣿⣿⣿⠿⠛⣋⣥⣴⡿⠇",
    "⢸⣿⢸⣿⡿⢿⣿⣿⠉⢻⣿⣷⣶⣶⣮⣭⣭⣛⣛⡻⠷⠾⠶⣶⣷⣾⣿⣿⣿⣿⣿⣿⣿⠿⠟⣋⣩⣴⠾⠛⠋⣁⢼⠀⠀",
    "⢸⣿⢸⣿⡄⢈⣹⣿⠀⢸⡿⠛⠛⠻⣿⠏⠉⣿⣿⣷⣶⣶⣦⣤⣬⣛⣛⡻⠿⠿⠿⢛⣩⣤⠶⠛⠋⠉⠀⠀⠀⠐⢺⡀⠀",
    "⠸⣿⣼⣿⣿⣾⣿⣿⠀⢸⡇⠀⠛⠀⢸⡆⠀⣤⡿⠛⠛⠻⣿⡿⠿⠿⣿⣿⣿⣿⣿⡟⣿⠀⠀⠀⠀⠀⠀⢀⡠⠞⢋⣿⡆",
    "⠀⠀⠉⠉⠛⠛⠿⠿⢶⣤⣷⣄⣉⣉⣹⣇⠀⠿⡇⠀⠛⠀⢸⠀⢰⡆⠈⣿⠏⠹⢿⡇⣿⠀⠀⠀⣀⠤⢊⣡⣴⠿⠛⠁⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠉⠙⠛⠛⠲⠶⠷⢄⣉⣉⣹⣆⠈⠃⣠⣿⣆⣰⣾⡇⣿⣠⠚⢉⣴⠾⠛⠉⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠉⠉⠛⠛⠻⠿⠿⢿⣿⣿⣿⣇⣩⣶⠿⠉⠁⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠉⠉⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
];

pub fn banner(width: u16, word: &str) -> Vec<String> {
    let letters = word.chars().count();
    let inner = usize::from(width).max(letters + 2).saturating_sub(2);
    let left = (inner - letters) / 2;
    vec![
        format!("╔{}╗", "═".repeat(inner)),
        format!(
            "║{}{word}{}║",
            " ".repeat(left),
            " ".repeat(inner - letters - left)
        ),
        format!("╚{}╝", "═".repeat(inner)),
    ]
}

const GRADIENT: [(u8, u8, u8); 5] = [
    (0xb4, 0xbe, 0xfe),
    (0x89, 0xb4, 0xfa),
    (0x74, 0xc7, 0xec),
    (0x89, 0xdc, 0xeb),
    (0x94, 0xe2, 0xd5),
];

pub fn band(row: usize, rows: usize) -> (u8, u8, u8) {
    if rows == 0 {
        return GRADIENT[0];
    }
    GRADIENT[(row * GRADIENT.len() / rows).min(GRADIENT.len() - 1)]
}

pub fn art_width(art: &[&str]) -> u16 {
    u16::try_from(art.first().map_or(0, |line| line.chars().count())).unwrap_or(u16::MAX)
}

pub fn art_height(art: &[&str]) -> u16 {
    u16::try_from(art.len()).unwrap_or(u16::MAX)
}

/// Every one of these is Wide in East_Asian_Width, so it takes two columns on
/// its own. The obvious alternatives are not: U+2713 CHECK MARK and U+1F5D1
/// WASTEBASKET need U+FE0F behind them to render as pictures at all, and
/// terminals disagree about how wide that makes them. A header that is one
/// column out in half the terminals it runs in is worse than no mark.
///
/// [`MARK_READING`] is the cat because the agent-facing protocol already
/// opens that same sentence with it; the rest are the action rather than the
/// animal, which is what tells them apart.
/// Five of these are cats, and the split is not decoration. A cat is used
/// wherever the animal *is* the meaning — taking something in, having nothing
/// to tend, catching what nobody asked it to catch, holding a thread. Where the
/// meaning is a clock, a search or a wire, the object says it and a cat face
/// would only say "this line is Sardi", which the name already says.
///
/// Nothing here may be a ZWJ sequence. 🐈‍⬛ is three codepoints, and both the
/// width and the first-character check below would read only the first of them.
const MARK_READING: &str = "🐈";
const MARK_ADOPTED: &str = "😻";
const MARK_LISTENING: &str = "🔔";
const MARK_AVAILABLE: &str = "🔌";
const MARK_IDLE: &str = "💤";
const MARK_WATCHING: &str = "👀";
const MARK_EMPTY: &str = "😿";
const MARK_NUDGE: &str = "⏰";
const MARK_REMEMBERS: &str = "📚";
const MARK_RESTORED: &str = "🧶";
const MARK_RECALLS: &str = "🔎";
const MARK_CAPTURED: &str = "😼";
const MARK_DUE: &str = "📖";

fn say(mark: &str, template: &str) -> String {
    format!("{mark} {}", crate::i18n::fill(template, "name", NAME))
}

pub fn reading(language: Interface) -> String {
    say(MARK_READING, voices::lines(language).reading)
}

pub fn adopted(language: Interface, count: i64) -> String {
    let lines = voices::lines(language);
    if count == 0 {
        return say(MARK_ADOPTED, lines.adopted_none);
    }
    counted(MARK_ADOPTED, &lines.adopted, language, count)
}

fn counted(mark: &str, wording: &voices::Counted, language: Interface, count: i64) -> String {
    let line = say(mark, wording.pick(language, count));
    crate::i18n::fill(&line, "count", count)
}

pub fn configured(language: Interface, agent: &str, with_hooks: bool) -> String {
    let lines = voices::lines(language);
    let (mark, template) = if with_hooks {
        (MARK_LISTENING, lines.listening)
    } else {
        (MARK_AVAILABLE, lines.available)
    };
    crate::i18n::fill(&say(mark, template), "agent", agent)
}

pub fn idle(language: Interface) -> String {
    say(MARK_IDLE, voices::lines(language).idle)
}

pub fn watching(language: Interface, memories: i64, projects: usize) -> String {
    let line = say(MARK_WATCHING, voices::lines(language).watching);
    let line = crate::i18n::fill(&line, "memories", memories);
    crate::i18n::fill(&line, "projects", projects)
}

pub fn empty(language: Interface) -> String {
    say(MARK_EMPTY, voices::lines(language).empty)
}

fn if_any(
    mark: &str,
    wording: &voices::Counted,
    language: Interface,
    count: i64,
) -> Option<String> {
    (count > 0).then(|| counted(mark, wording, language, count))
}

pub fn remembers(language: Interface, count: i64) -> Option<String> {
    if_any(
        MARK_REMEMBERS,
        &voices::lines(language).remembers,
        language,
        count,
    )
}

pub fn due(language: Interface, count: i64) -> Option<String> {
    if_any(MARK_DUE, &voices::lines(language).due, language, count)
}

pub fn restored(language: Interface, count: usize) -> Option<String> {
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    if_any(
        MARK_RESTORED,
        &voices::lines(language).restored,
        language,
        count,
    )
}

pub fn recalls(language: Interface, count: usize) -> Option<String> {
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    if_any(
        MARK_RECALLS,
        &voices::lines(language).recalls,
        language,
        count,
    )
}

pub fn captured(language: Interface, count: usize) -> Option<String> {
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    if_any(
        MARK_CAPTURED,
        &voices::lines(language).captured,
        language,
        count,
    )
}

pub fn span(language: Interface, minutes: i64) -> String {
    let lines = voices::lines(language);
    let (counted, count) = match minutes {
        ..90 => (&lines.minutes, minutes),
        90..2880 => (&lines.hours, minutes / 60),
        _ => (&lines.days, minutes / 1440),
    };
    crate::i18n::fill(counted.pick(language, count), "count", count)
}

pub fn nudge(language: Interface, project: &str, quiet_minutes: i64) -> String {
    let lines = voices::lines(language);
    let span = span(language, quiet_minutes);
    let line = say(MARK_NUDGE, lines.nudge);
    let line = crate::i18n::fill(&line, "project", format!("{project:?}"));
    crate::i18n::fill(&line, "span", span)
}

#[cfg(test)]
mod tests {
    use super::*;

    use Interface::{English, Spanish};

    #[test]
    fn a_span_is_said_in_the_largest_unit_that_still_reads() {
        assert_eq!(span(English, 1), "a minute");
        assert_eq!(span(English, 45), "45 minutes");
        assert_eq!(span(English, 89), "89 minutes");
        assert_eq!(
            span(English, 90),
            "an hour",
            "the plural table reads better than the number"
        );
        assert_eq!(span(English, 240), "4 hours");
        assert_eq!(span(English, 2879), "47 hours");
        assert_eq!(span(English, 2880), "2 days");
        assert_eq!(span(English, 7504), "5 days");
        assert_eq!(span(Spanish, 7504), "5 días");

        for language in Interface::ALL {
            for minutes in [0, 1, 59, 89, 90, 1439, 2880, 7504, 100_000] {
                let said = span(language, minutes);
                assert!(
                    !said.contains("{count}"),
                    "{language:?} at {minutes}: {said}"
                );
                assert!(!said.is_empty(), "{language:?} at {minutes}");
            }
            assert_ne!(
                span(language, 7504),
                span(language, 7504 * 2),
                "{language:?} says the same thing for five days and for ten"
            );
        }
    }

    #[test]
    fn counts_read_naturally_at_one_and_at_none() {
        assert_eq!(adopted(English, 1), "😻 Sardi kept 1 memory.");
        assert_eq!(adopted(English, 3), "😻 Sardi kept 3 memories.");
        assert_eq!(adopted(English, 0), "😻 Sardi found nothing worth keeping.");

        assert_eq!(adopted(Spanish, 1), "😻 Sardi guardó 1 memoria.");
        assert_eq!(adopted(Spanish, 3), "😻 Sardi guardó 3 memorias.");
        assert_eq!(
            adopted(Spanish, 0),
            "😻 Sardi no encontró nada que valiera la pena guardar."
        );
    }

    /// Every line a person is meant to read, in one place, so the checks below
    /// cannot pass by testing a subset of the voice.
    ///
    /// Takes the language so the structural checks — a mark on every line, a
    /// name in every line, no two lines sharing a mark — run against each one.
    /// A translation that dropped a mark, or pasted the wrong one, would
    /// otherwise ship on the strength of the English having been right.
    fn every_line(language: Interface) -> Vec<String> {
        vec![
            reading(language),
            adopted(language, 2),
            adopted(language, 0),
            configured(language, "Claude Code", true),
            configured(language, "Codex", false),
            idle(language),
            watching(language, 3312, 16),
            empty(language),
            nudge(language, "leteo", 45),
            remembers(language, 2).expect("a count of two is not silence"),
            restored(language, 2).expect("a count of two is not silence"),
            recalls(language, 2).expect("a count of two is not silence"),
            captured(language, 2).expect("a count of two is not silence"),
            due(language, 2).expect("a count of two is not silence"),
        ]
    }

    #[test]
    fn the_lines_a_hook_shows_say_nothing_at_zero() {
        for language in Interface::ALL {
            assert_eq!(remembers(language, 0), None, "{language:?}");
            assert_eq!(restored(language, 0), None, "{language:?}");
            assert_eq!(recalls(language, 0), None, "{language:?}");
            assert_eq!(captured(language, 0), None, "{language:?}");
            assert_eq!(due(language, 0), None, "{language:?}");
            assert_eq!(
                due(language, -1),
                None,
                "a failed count is not a queue of -1: {language:?}"
            );
        }

        assert!(
            recalls(English, 1)
                .expect("one note is not silence")
                .starts_with("🔎 Sardi has a note that might fit.")
        );
        assert!(
            captured(English, 3)
                .expect("three memories are not silence")
                .starts_with("😼 Sardi kept 3 memories from that subagent.")
        );
        assert!(
            recalls(Spanish, 1)
                .expect("one note is not silence")
                .starts_with("🔎 Sardi tiene una nota que podría encajar.")
        );
    }

    #[test]
    fn every_line_names_the_character() {
        for language in Interface::ALL {
            for line in every_line(language) {
                assert!(
                    line.contains(NAME),
                    "line without a speaker in {language:?}: {line}"
                );
            }
        }
    }

    #[test]
    fn no_language_is_left_speaking_another_one() {
        for line in every_line(Spanish) {
            for english_only in [" is ", " has ", " kept ", " nothing ", " memories"] {
                assert!(
                    !line.contains(english_only),
                    "a Spanish line still carries {english_only:?}: {line}"
                );
            }
        }
        for line in every_line(English) {
            for spanish_only in [" está ", " tiene ", " guardó ", " memorias"] {
                assert!(
                    !line.contains(spanish_only),
                    "an English line still carries {spanish_only:?}: {line}"
                );
            }
        }

        for (index, first) in Interface::ALL.iter().enumerate() {
            for second in &Interface::ALL[index + 1..] {
                for (slot, (left, right)) in voices::lines(*first)
                    .sentences()
                    .into_iter()
                    .zip(voices::lines(*second).sentences())
                    .enumerate()
                {
                    assert_ne!(
                        left, right,
                        "{first:?} and {second:?} share sentence {slot}, so one is speaking the \
                         other's language; see src/sardi/voices.rs"
                    );
                }
                assert_ne!(
                    voices::lines(*first).spans(),
                    voices::lines(*second).spans(),
                    "{first:?} and {second:?} measure time in the same words throughout"
                );
            }
        }
    }

    #[test]
    fn the_name_still_ends_the_way_the_basque_lines_assume() {
        let last = NAME.chars().last().expect("the name is not empty");
        assert!(
            "aeiou".contains(last.to_ascii_lowercase()),
            "{NAME} ends in {last:?}, so the Basque lines need -ek where they \
             currently say -k; see src/sardi/voices.rs"
        );
    }

    #[test]
    fn polish_counts_in_three_forms_and_everything_else_in_two() {
        use crate::settings::Interface::{English, Polish};

        assert!(adopted(Polish, 1).ends_with("1 wspomnienie."));
        assert!(adopted(Polish, 2).ends_with("2 wspomnienia."));
        assert!(adopted(Polish, 5).ends_with("5 wspomnień."));
        assert!(adopted(Polish, 12).ends_with("12 wspomnień."), "the teens");
        assert!(adopted(Polish, 22).ends_with("22 wspomnienia."));
        assert!(adopted(Polish, 25).ends_with("25 wspomnień."));

        for language in Interface::ALL {
            if language == Polish {
                continue;
            }
            let two = adopted(language, 2);
            let five = adopted(language, 5);
            assert_eq!(
                two.replace('2', "#"),
                five.replace('5', "#"),
                "{language:?} has grown a third form nothing chooses"
            );
        }
        assert!(adopted(English, 2).ends_with("2 memories."));
    }

    #[test]
    fn every_line_opens_with_a_mark_of_its_own() {
        for language in Interface::ALL {
            let mut seen: std::collections::BTreeMap<char, usize> =
                std::collections::BTreeMap::new();
            for line in every_line(language) {
                let mark = line.chars().next().expect("a line is never empty");
                assert!(
                    is_wide(mark),
                    "{line:?} opens with {mark:?}, which is not a two-column mark"
                );
                assert!(
                    line[mark.len_utf8()..].starts_with(' '),
                    "the mark runs into the sentence: {line}"
                );
                *seen.entry(mark).or_default() += 1;
            }
            let shared: Vec<char> = seen
                .iter()
                .filter(|(_, times)| **times > 1)
                .map(|(mark, _)| *mark)
                .collect();
            assert_eq!(
                shared,
                vec![MARK_ADOPTED.chars().next().unwrap()],
                "two lines share a mark in {language:?}: {seen:?}"
            );
        }
    }

    fn is_wide(character: char) -> bool {
        matches!(character as u32, 0x23F0..=0x23F3 | 0x1F300..=0x1FAFF)
    }

    #[test]
    fn every_drawing_is_a_rectangle_of_braille() {
        let width = CAT_LARGE[0].chars().count();
        for (index, line) in CAT_LARGE.iter().enumerate() {
            assert_eq!(
                line.chars().count(),
                width,
                "line {index} is a different width: {line}"
            );
            for character in line.chars() {
                assert!(
                    ('\u{2800}'..='\u{28FF}').contains(&character),
                    "line {index} holds {character:?}, not a Braille pattern"
                );
            }
        }
        assert_eq!(art_width(CAT_LARGE), width as u16);
        assert_eq!(art_height(CAT_LARGE), CAT_LARGE.len() as u16);
    }

    #[test]
    fn the_drawing_is_not_blank() {
        let lit = CAT_LARGE
            .iter()
            .flat_map(|line| line.chars())
            .filter(|character| *character != '\u{2800}')
            .count();
        assert!(
            lit > CAT_LARGE.len() * 4,
            "the drawing is too empty to be a cat"
        );
    }

    #[test]
    fn the_banner_matches_the_drawing_it_sits_under() {
        let width = art_width(CAT_LARGE);
        let lines = banner(width, "LETEO");
        for line in &lines {
            assert_eq!(
                u16::try_from(line.chars().count()).unwrap(),
                width,
                "a banner that is not the width of the art will not stack with it: {line}"
            );
        }
        assert!(lines[1].contains("LETEO"));
        assert!(lines[0].starts_with('╔') && lines[0].ends_with('╗'));
        assert!(lines[2].starts_with('╚') && lines[2].ends_with('╝'));
    }

    #[test]
    fn a_banner_narrower_than_its_word_grows_to_fit() {
        let lines = banner(2, "LETEO");
        assert_eq!(lines[1], "║LETEO║");
        assert_eq!(lines[0].chars().count(), lines[1].chars().count());
    }

    #[test]
    fn the_gradient_spans_the_whole_drawing() {
        let rows = CAT_LARGE.len();
        assert_eq!(band(0, rows), GRADIENT[0], "the top row starts the ramp");
        assert_eq!(
            band(rows - 1, rows),
            GRADIENT[GRADIENT.len() - 1],
            "and the bottom row finishes it"
        );
        let used: std::collections::BTreeSet<_> = (0..rows).map(|r| band(r, rows)).collect();
        assert_eq!(
            used.len(),
            GRADIENT.len(),
            "every band should appear across {rows} rows, saw {}",
            used.len()
        );
    }

    #[test]
    fn banding_a_drawing_with_no_rows_does_not_divide_by_zero() {
        assert_eq!(band(0, 0), GRADIENT[0]);
    }

    #[test]
    fn no_drawing_carries_blank_rows_at_its_edges() {
        let blank = |line: &&str| line.chars().all(|c| c == '\u{2800}');
        assert!(
            !CAT_LARGE.first().is_some_and(blank),
            "the drawing starts with a blank row"
        );
        assert!(
            !CAT_LARGE.last().is_some_and(blank),
            "the drawing ends with a blank row"
        );
    }

    #[test]
    fn the_drawing_reports_its_own_size() {
        assert_eq!(art_width(CAT_LARGE), 45);
        assert_eq!(art_height(CAT_LARGE), 29);
        assert_eq!(art_width(&[]), 0);
        assert_eq!(art_height(&[]), 0);
    }

    #[test]
    fn the_protocol_agents_receive_teaches_the_voice() {
        let protocol = crate::setup::MEMORY_PROTOCOL;
        assert!(protocol.contains(NAME), "the protocol must name the cat");
        assert!(
            protocol.contains("Never put it in an error"),
            "the protocol must keep failures free of the mascot"
        );
    }

    #[test]
    fn the_nudge_stays_a_usable_instruction() {
        let line = nudge(English, "leteo", 45);
        assert!(
            line.starts_with(
                "⏰ Sardi has been given nothing to keep for \"leteo\" in 45 minutes."
            ),
            "{line}"
        );
        assert!(
            nudge(English, "leteo", 1).contains("in a minute."),
            "{}",
            nudge(English, "leteo", 1)
        );

        for language in Interface::ALL {
            let line = nudge(language, "leteo", 45);
            assert!(
                line.contains("mem_save"),
                "the reminder must still name the call in {language:?}: {line}"
            );
            assert!(line.contains("\"leteo\""), "{line}");
            assert!(line.contains("45"), "{line}");
        }
        assert!(
            nudge(Spanish, "leteo", 1).contains("en un minuto."),
            "{}",
            nudge(Spanish, "leteo", 1)
        );
    }

    #[test]
    fn hooks_change_what_is_promised() {
        assert_eq!(
            configured(English, "Claude Code", true),
            "🔔 Sardi will be listening in Claude Code."
        );
        assert_eq!(
            configured(English, "Claude Code", false),
            "🔌 Sardi is available in Claude Code."
        );
        for language in Interface::ALL {
            let listening = configured(language, "Claude Code", true);
            let available = configured(language, "Claude Code", false);
            assert_ne!(listening, available, "{language:?}");
            assert!(listening.starts_with(MARK_LISTENING), "{listening}");
            assert!(available.starts_with(MARK_AVAILABLE), "{available}");
        }
    }
}
