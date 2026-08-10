//! Sardi, the voice Leteo speaks with.
//!
//! Leteo is the store; Sardi is who tends it. Saying who did the work reads
//! better than reporting a count: "Sardi kept 8 memories" is somebody making a
//! decision, where "8 observations written" is a number.
//!
//! # Where this belongs
//!
//! Wherever a person is reading: the setup wizard, the terminal UI, and the
//! system message a hook shows mid-session. The CLI answers in JSON because
//! scripts parse it, and a character has no place in a machine-readable
//! contract — a caller doing `leteo stats | jq` must never have to skip past a
//! cat. MCP responses are the same contract and stay just as plain.
//!
//! # Speaking through an agent
//!
//! Most people never see this module. They talk to Leteo through an agent,
//! which reads tool responses and puts the result in its own words, so nothing
//! written here would reach them. The voice travels as an instruction instead:
//! [`crate::setup::MEMORY_PROTOCOL`] tells the agent to attribute memory work
//! to Sardi, and the session-start hook injects it. That keeps the stored data
//! and the tool contract plain while the character survives the paraphrase.
//!
//! # Which language it says it in
//!
//! Every line here takes an [`Interface`] and returns that language's version
//! of the sentence, because a memory tool that greets somebody in a language
//! they did not choose is a memory tool that did not read its own settings.
//! The marks, the counts and the name are shared; only the words change.
//!
//! The language is passed in rather than read from a global. A process does
//! have exactly one answer, so a global would work and would also make every
//! test that touches the voice race every other one — the same trap
//! [`crate::settings::language_for_locale`] was split out of.
//!
//! # Where it must not soften anything
//!
//! Failures keep their own words. A refusal has to stay precise and
//! actionable, and there is nothing charming about a mascot standing between
//! somebody and the thing they need to fix. The vocabulary here covers work
//! that went well; errors are reported plainly, as they were before.

mod voices;

use crate::settings::Interface;

/// The character's name, in one place so it can be changed in one place.
///
/// Not translated, and that is the point of a name. Sardi is Sardi in every
/// language; what changes around it is the sentence.
pub const NAME: &str = "Sardi";

/// Sardi in full, in Braille.
///
/// A Braille character carries a 2-wide by 4-tall grid of dots, so a line of
/// them is four times the vertical resolution of block characters, and the dots
/// come out square: a cell is about twice as tall as it is wide, and the grid
/// halves it in both directions. That is what lets a curve read as a curve
/// rather than a staircase.
///
/// Converted from an image rather than generated. Earlier versions here were
/// built out of ellipses and strokes, and every feature added made the next one
/// harder to read until the face was hatching; an image buys detail that
/// geometry at this scale cannot.
///
/// Frozen as characters rather than kept as a bitmap with a converter: the art
/// does not change at runtime, so shipping the geometry that produced it would
/// be code that runs once and never differs.
///
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
    "⠸⣿⣼⣿⣿⣾⣿⣿⠀⢸⡇⠀⠛⠀⢸⡆⠀⣤⡟⠉⣉⠙⢿⡿⠿⠿⣿⣿⣿⣿⣿⡟⣿⠀⠀⠀⠀⠀⠀⢀⡠⠞⢋⣿⡆",
    "⠀⠀⠉⠉⠛⠛⠿⠿⢶⣤⣷⣄⣉⣉⣹⣇⠀⠿⡇⠀⣉⣀⣸⠀⢰⡆⠈⣿⠏⠹⢿⡇⣿⠀⠀⠀⣀⠤⢊⣡⣴⠿⠛⠁⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠉⠙⠛⠛⠲⠶⠿⢦⣤⣄⣼⣆⠈⠃⣠⣿⣆⣰⣾⡇⣿⣠⠚⢉⣴⠾⠛⠉⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠉⠉⠛⠛⠻⠿⠿⢿⣿⣿⣿⣇⣩⣶⠿⠉⠁⠀⠀⠀⠀⠀⠀⠀⠀",
    "⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠉⠉⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀",
];

/// A boxed wordmark to stack under a drawing.
///
/// Built to the drawing's width rather than written out, so the two always
/// line up: a banner hardcoded at one width silently stops matching the first
/// time the art beside it is redrawn.
pub fn banner(width: u16, word: &str) -> Vec<String> {
    let letters = word.chars().count();
    // A box narrower than what it holds would render with the word hanging out
    // of its own frame.
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

/// The colours a drawing is banded into, from its top row to its bottom.
///
/// Five bands rather than a colour per row: the steps land on features at this
/// size, and a smooth per-row ramp over twenty-nine rows is a change of two or
/// three points at a time, which reads as one flat colour.
///
/// The hues run through the cyan the header and the counters already use, so
/// the drawing belongs to the interface around it rather than sitting on it.
const GRADIENT: [(u8, u8, u8); 5] = [
    (0xb4, 0xbe, 0xfe),
    (0x89, 0xb4, 0xfa),
    (0x74, 0xc7, 0xec),
    (0x89, 0xdc, 0xeb),
    (0x94, 0xe2, 0xd5),
];

/// The colour for one row of a drawing, as red, green and blue.
///
/// Returned as components rather than a terminal colour so this module stays
/// clear of the UI crate: the wizard and the CLI use the rest of it, and
/// neither should have to link a renderer to ask Sardi's name.
pub fn band(row: usize, rows: usize) -> (u8, u8, u8) {
    if rows == 0 {
        return GRADIENT[0];
    }
    GRADIENT[(row * GRADIENT.len() / rows).min(GRADIENT.len() - 1)]
}

/// How wide a drawing is on screen, in columns.
pub fn art_width(art: &[&str]) -> u16 {
    u16::try_from(art.first().map_or(0, |line| line.chars().count())).unwrap_or(u16::MAX)
}

/// How tall a drawing is on screen, in rows.
pub fn art_height(art: &[&str]) -> u16 {
    u16::try_from(art.len()).unwrap_or(u16::MAX)
}

/// The mark that opens a line, one per kind of work.
///
/// A mark per action lets somebody sort a screen of output before reading a
/// word of it. One shared mark would only say "this line is Sardi", and the
/// name in the sentence already says that.
///
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

/// Fills a line's placeholders and puts its mark in front.
///
/// One place where the name goes in, one place where the mark goes on. Every
/// line below is a sentence with `{name}` somewhere inside it rather than a
/// prefix and a tail, because word order decides where the name falls and not
/// every language puts it first — Spanish opens the reminder with "a Sardi",
/// where English opens it with "Sardi has".
fn say(mark: &str, template: &str) -> String {
    format!("{mark} {}", crate::i18n::fill(template, "name", NAME))
}

/// Reading the store before anything has been decided.
pub fn reading(language: Interface) -> String {
    say(MARK_READING, voices::lines(language).reading)
}

/// Memories taken over from somewhere else.
///
/// The word is adoption and so is the mark: these memories were somebody
/// else's store and are now being taken in and kept.
pub fn adopted(language: Interface, count: i64) -> String {
    let lines = voices::lines(language);
    if count == 0 {
        return say(MARK_ADOPTED, lines.adopted_none);
    }
    counted(MARK_ADOPTED, &lines.adopted, language, count)
}

/// A line whose wording depends on how many, with the number filled in.
fn counted(mark: &str, wording: &voices::Counted, language: Interface, count: i64) -> String {
    let line = say(mark, wording.pick(language, count));
    crate::i18n::fill(&line, "count", count)
}

/// An agent wired up and ready.
///
/// The two answers get different marks because they promise different things:
/// one is listening on its own, the other waits to be asked.
pub fn configured(language: Interface, agent: &str, with_hooks: bool) -> String {
    let lines = voices::lines(language);
    let (mark, template) = if with_hooks {
        (MARK_LISTENING, lines.listening)
    } else {
        (MARK_AVAILABLE, lines.available)
    };
    crate::i18n::fill(&say(mark, template), "agent", agent)
}

/// Nothing was asked of it.
pub fn idle(language: Interface) -> String {
    say(MARK_IDLE, voices::lines(language).idle)
}

/// What the store currently holds, for a status line.
pub fn watching(language: Interface, memories: i64, projects: usize) -> String {
    let line = say(MARK_WATCHING, voices::lines(language).watching);
    let line = crate::i18n::fill(&line, "memories", memories);
    crate::i18n::fill(&line, "projects", projects)
}

/// A store that has nothing in it yet.
///
/// Lives here rather than being built at the call site so this line carries a
/// mark like every other. Assembling one sentence of the voice out of [`NAME`]
/// somewhere else is how it ends up as the only bare one on screen.
pub fn empty(language: Interface) -> String {
    say(MARK_EMPTY, voices::lines(language).empty)
}

/// A line a hook shows mid-conversation, or nothing at all.
///
/// The five below return nothing at zero rather than announcing that nothing
/// happened. A hook runs whether or not there was anything to do — on every
/// prompt, on every subagent — so a line that always rendered would be
/// narrating the machinery instead of the work, which is the one thing this
/// voice must not become. The rule is kept here rather than at each call site
/// so that a new caller cannot forget it, and a count below zero is a failed
/// count rather than a queue, so it is silent too.
fn if_any(
    mark: &str,
    wording: &voices::Counted,
    language: Interface,
    count: i64,
) -> Option<String> {
    (count > 0).then(|| counted(mark, wording, language, count))
}

/// What the project already holds, said once as a session opens.
pub fn remembers(language: Interface, count: i64) -> Option<String> {
    if_any(
        MARK_REMEMBERS,
        &voices::lines(language).remembers,
        language,
        count,
    )
}

/// Pairs the store has proposed and nobody has ruled on.
///
/// Leteo finds candidate conflicts on the way in and offers them to the agent
/// in the reply to `mem_save`. If that turn ends without a verdict, nothing
/// ever mentions them again: a real store had seventy of them, the oldest
/// eight weeks old, and the only way to find out was to go looking.
///
/// The mark is two pieces that may or may not fit, which is the question.
/// Memories whose review window has come round.
///
/// Beside `waiting` and for the same reason it is said there and nowhere else.
/// A review date is set when a decision, a policy or a preference is saved, and
/// migration 15 rewrote every one of them on this store — 269 memories carry
/// one. Nothing ever mentioned the queue: `mem_review` reads it, the skill
/// lists that tool without ever saying when to reach for it, no hook named it,
/// and the command line has no equivalent at all. A window that nothing opens
/// is a window that does not exist, which is the same defect `policy` had when
/// its own window could never fire.
///
/// A session opening, once, like the verdicts: at every prompt it would nag,
/// and never is where it was.
pub fn due(language: Interface, count: i64) -> Option<String> {
    if_any(MARK_DUE, &voices::lines(language).due, language, count)
}

/// What survived a context compaction.
///
/// The sentence is about holding a thread and the mark is the thread, which is
/// the one place where the cat and the meaning are the same object.
pub fn restored(language: Interface, count: usize) -> Option<String> {
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    if_any(
        MARK_RESTORED,
        &voices::lines(language).restored,
        language,
        count,
    )
}

/// Earlier work that may match what was just asked.
///
/// Hedged on purpose, and the hedge is measured rather than modest: the search
/// behind it hands over the right memory on 22% of the prompts it speaks on,
/// for the reasons written at [`crate::hooks`]. "Sardi remembers three notes
/// about this" would be a claim about the answer; what is true is that three
/// things came back and one of them might help.
pub fn recalls(language: Interface, count: usize) -> Option<String> {
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    if_any(
        MARK_RECALLS,
        &voices::lines(language).recalls,
        language,
        count,
    )
}

/// Memories taken from a subagent's report without being asked.
///
/// Nobody requested this one: the subagent was reporting to somebody else and
/// Sardi took what was worth keeping on the way past. The pleased-with-itself
/// cat is the register that says so.
pub fn captured(language: Interface, count: usize) -> Option<String> {
    let count = i64::try_from(count).unwrap_or(i64::MAX);
    if_any(
        MARK_CAPTURED,
        &voices::lines(language).captured,
        language,
        count,
    )
}

/// A reminder that nothing has been saved in a while.
///
/// This one is read by both the agent and the person: it is shown as a system
/// message and it has to remain a usable instruction, so the mark and the
/// voice go in front of the ask rather than in place of it.
/// A length of time in the largest unit that still reads as one.
///
/// A reminder that says 7,504 minutes is arithmetically right and tells a
/// reader nothing; that number came off a real store, from a project untouched
/// for five days. Minutes up to an hour and a half, hours up to two days, days
/// after that — the thresholds are past the point where the smaller unit stops
/// being read rather than at the exact conversion, because "90 minutes" is
/// still a length of time and "36 hours" still beats "a day and a half".
///
/// The count is floored, so it never claims more time than has passed.
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
    // Quoted the way it always was: a project name with a space in it has to
    // read as one name rather than as the end of the sentence.
    let line = crate::i18n::fill(&line, "project", format!("{project:?}"));
    crate::i18n::fill(&line, "span", span)
}

#[cfg(test)]
mod tests {
    use super::*;

    use Interface::{English, Spanish};

    /// A length of time is said in a unit somebody can read.
    ///
    /// The reminder announced 7,504 minutes on a real store — arithmetically
    /// right, and five days of nothing having happened. Every language gets
    /// the same treatment, because "7504 minuten" is the same defect in
    /// another tongue and the line above would not catch it.
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

        // Never more time than has passed, and never a number where a word
        // reads better.
        for language in Interface::ALL {
            for minutes in [0, 1, 59, 89, 90, 1439, 2880, 7504, 100_000] {
                let said = span(language, minutes);
                assert!(
                    !said.contains("{count}"),
                    "{language:?} at {minutes}: {said}"
                );
                assert!(!said.is_empty(), "{language:?} at {minutes}");
            }
            // The unit changes rather than the number growing without bound.
            assert_ne!(
                span(language, 7504),
                span(language, 7504 * 2),
                "{language:?} says the same thing for five days and for ten"
            );
        }
    }

    #[test]
    fn counts_read_naturally_at_one_and_at_none() {
        // A line that says "1 memories" undoes the whole point of having a
        // voice, and "kept 0 memories" is a worse way of saying nothing
        // happened.
        assert_eq!(adopted(English, 1), "😻 Sardi kept 1 memory.");
        assert_eq!(adopted(English, 3), "😻 Sardi kept 3 memories.");
        assert_eq!(adopted(English, 0), "😻 Sardi found nothing worth keeping.");

        // And in every other language, because "1 memorias" is the same defect
        // in a different tongue and nothing above would catch it.
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
    /// Every line, and *every* is load-bearing. `due` and `adopted_none` were
    /// missing from this list, so the checks below ran on eleven of thirteen
    /// sentences while claiming to run on the voice — and the Swedish `due` sat
    /// in Polish, word for word `POLISH.due`, until somebody read the table for
    /// an unrelated reason. A line added to [`Lines`] and not added here is a
    /// line nothing under this comment is looking at.
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
        // A hook fires whether or not it had anything to do. These four run on
        // every prompt and every subagent, so "nothing happened" has to come
        // back as no line at all rather than as a line saying so.
        //
        // Checked in every language: silence at zero is decided in the same
        // match that picks the words, so a language added carelessly is exactly
        // where a zero would start speaking again.
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
        // The voice is worth nothing if half the lines are anonymous.
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
        // The failure a pair of parallel `match` arms invites: a variant added
        // by copying its neighbour and then only half rewritten. Neither the
        // mark checks nor the name check would notice, because a line left in
        // English has both.
        //
        // Anchored on words that only belong to one of the two, rather than on
        // whole sentences, so rewording a line does not fail this.
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

        // And the general form, which needs no word list at all: no two of
        // these tables hold the same sentence in the same slot.
        //
        // Read off the tables rather than off rendered lines, and slot by slot
        // across every pair rather than whole-set against English. Both halves
        // were wrong before and each one alone let the Swedish `due` through:
        // English was never the table it was copied from, and rendered at a
        // count of two Polish takes its `few` form while Swedish takes `many`,
        // so the same words came out looking different. `Lines::sentences`
        // compares all three forms of every counted line.
        //
        // Two languages are free to phrase *different* sentences alike; what is
        // asserted is only that the same slot is not filled with the same words
        // twice. Close relatives are the test of that — Spanish against
        // Galician, Portuguese against Galician, Catalan against Spanish — and
        // every slot differs in each of those pairs.
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
                // The lengths of time are held to the weaker rule, because
                // "un minuto" is correct in both Spanish and Galician and a
                // slot-by-slot check would call that a defect.
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
        // Basque marks the subject of a sentence like these on the name itself,
        // and which ending it takes depends on the last letter: a vowel takes
        // `-k`, a consonant takes `-ek`. [`NAME`] is a constant, so the answer
        // is known and the lines say `{name}k` — "Sardik" — rather than the
        // `{name}(e)k` that a template writes when it does not know, and which
        // reaches the screen as "Sardi(e)k".
        //
        // The cost of committing is this assumption, so it is checked rather
        // than trusted: renaming the character to something ending in a
        // consonant would make every Basque line ungrammatical, and nothing
        // else here would notice — the marks would be right, the name would be
        // present, and the sentence would simply be wrong for its readers.
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

        // 1 wspomnienie, 2 wspomnienia, 5 wspomnień — and 12 goes back to the
        // last of those, which is the part of the rule that gets left out. A
        // language with three forms fed through a two-form table says
        // "2 wspomnień", which reads the way "2 memory" does.
        assert!(adopted(Polish, 1).ends_with("1 wspomnienie."));
        assert!(adopted(Polish, 2).ends_with("2 wspomnienia."));
        assert!(adopted(Polish, 5).ends_with("5 wspomnień."));
        assert!(adopted(Polish, 12).ends_with("12 wspomnień."), "the teens");
        assert!(adopted(Polish, 22).ends_with("22 wspomnienia."));
        assert!(adopted(Polish, 25).ends_with("25 wspomnień."));

        // Everywhere else the middle form is the plural one, so a count of two
        // and a count of five read alike.
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
        // Two actions sharing a mark is the same as having no mark, and a
        // pasted constant does that without changing anything else about the
        // line it is in.
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
                // The two answers `configured` gives are one action wearing two
                // marks on purpose, so they are checked as a pair below rather
                // than counted as a collision here.
                *seen.entry(mark).or_default() += 1;
            }
            // Exactly one collision is by design and it is named, rather than
            // allowed for by a count: `adopted` and `adopted_none` are one
            // action with two sentences — what was kept, and that nothing was —
            // so they share a mark. Any other repeat is a pasted constant, and
            // a subtracted number would have hidden it the moment two defects
            // cancelled out.
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

    /// The East_Asian_Width=Wide blocks the marks are drawn from.
    ///
    /// Anything outside these may need U+FE0F to render as a picture, and a
    /// mark whose width the terminal guesses at will shift the line it opens.
    fn is_wide(character: char) -> bool {
        matches!(character as u32, 0x23F0..=0x23F3 | 0x1F300..=0x1FAFF)
    }

    #[test]
    fn every_drawing_is_a_rectangle_of_braille() {
        // Ragged lines shear the picture, and a plain space is a different
        // width from a blank Braille cell in some terminals, which does the
        // same thing. Both are invisible in a diff.
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
        // A picture of entirely empty cells passes every check above and draws
        // nothing at all.
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
        // Asked for less room than the word needs, the frame has to widen.
        // Obeying would print the word hanging out of its own box.
        let lines = banner(2, "LETEO");
        assert_eq!(lines[1], "║LETEO║");
        assert_eq!(lines[0].chars().count(), lines[1].chars().count());
    }

    #[test]
    fn the_gradient_spans_the_whole_drawing() {
        // The point of banding is that the top and the bottom differ. An
        // off-by-one in the index arithmetic would quietly leave the last band
        // unused and the drawing would end one shade early.
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
        // Art converted from an image arrives padded, and the padding is not
        // visible in a diff: it is Braille either way. A blank row top or
        // bottom pushes the drawing off centre and eats one of the rows the
        // caption needs.
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
        // An empty drawing must not report a width from nowhere: the layout
        // subtracts it from the panel and would place things off screen.
        assert_eq!(art_width(&[]), 0);
        assert_eq!(art_height(&[]), 0);
    }

    #[test]
    fn the_protocol_agents_receive_teaches_the_voice() {
        // Agents learn Sardi from the protocol the session-start hook injects
        // and `leteo setup` writes into instruction files. Without this the
        // character exists only in the wizard, which a person sees once.
        let protocol = crate::setup::MEMORY_PROTOCOL;
        assert!(protocol.contains(NAME), "the protocol must name the cat");
        assert!(
            protocol.contains("Never put it in an error"),
            "the protocol must keep failures free of the mascot"
        );
    }

    #[test]
    fn the_nudge_stays_a_usable_instruction() {
        // It is shown to the person and read by the agent at the same time, so
        // the voice goes in front of the ask and never replaces it.
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

        // The ask itself, in every language. The reminder is the one line that
        // does work rather than reports it — a translation that kept the voice
        // and dropped the instruction would leave an agent nothing to act on,
        // and every other check here would still pass.
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
        // Without hooks Leteo only answers when asked; with them it is
        // watching the session. Saying the same thing for both would promise
        // something that does not happen.
        assert_eq!(
            configured(English, "Claude Code", true),
            "🔔 Sardi will be listening in Claude Code."
        );
        assert_eq!(
            configured(English, "Claude Code", false),
            "🔌 Sardi is available in Claude Code."
        );
        // The distinction is a promise, not a turn of phrase, so it has to
        // survive translation: two marks, and two different sentences.
        for language in Interface::ALL {
            let listening = configured(language, "Claude Code", true);
            let available = configured(language, "Claude Code", false);
            assert_ne!(listening, available, "{language:?}");
            assert!(listening.starts_with(MARK_LISTENING), "{listening}");
            assert!(available.starts_with(MARK_AVAILABLE), "{available}");
        }
    }
}
