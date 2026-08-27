//! Preferences that outlive a run, kept beside the store.
//!
//! There is exactly one of these so far, and the file exists rather than the
//! setting living in the database because a person has to be able to change it
//! without Leteo's help: a memory tool that has started talking too much is
//! answered by opening a file, not by learning a subcommand.
//!
//! Everything here reads on the critical path of somebody's conversation. A
//! missing, empty, truncated or hand-edited file therefore means the defaults
//! and never an error — a preference that cannot be read is a preference nobody
//! set, and refusing to run a hook over it would trade a cosmetic setting for
//! the memory itself.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Voice {
    /// Everything: what the project holds, what might fit, what was captured,
    /// what survived a compaction, and the save reminder.
    #[default]
    All,
    /// The save reminder alone.
    Reminders,
    /// Nothing at all.
    Quiet,
}

impl Voice {
    pub fn reports(self) -> bool {
        matches!(self, Self::All)
    }

    /// Whether the save reminder is shown.
    ///
    /// Deliberately a separate question from [`Voice::reports`]. The reminder
    /// is not a report — it is an instruction with something to do about it,
    /// and switching it off is what stops an agent being told to save at all.
    /// Folded into one flag, somebody who only wanted less chatter would lose
    /// the single line that does work, which is why [`Voice::Reminders`] sits
    /// between the two extremes rather than the setting being a boolean.
    pub fn reminders(self) -> bool {
        matches!(self, Self::All | Self::Reminders)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Reminders => "reminders",
            Self::Quiet => "quiet",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "all" => Some(Self::All),
            "reminders" => Some(Self::Reminders),
            "quiet" => Some(Self::Quiet),
            _ => None,
        }
    }

    /// The levels in the order they are offered, loudest first.
    pub const ALL: [Self; 3] = [Self::All, Self::Reminders, Self::Quiet];

    /// What this level promises, for the line beside it on screen.
    ///
    /// The words are in [`crate::i18n`] with every other screen's; what belongs
    /// here is only which of them goes with which level.
    pub fn description(self, language: Interface) -> &'static str {
        let say = crate::i18n::screens(language);
        match self {
            Self::All => say.voice_all,
            Self::Reminders => say.voice_reminders,
            Self::Quiet => say.voice_quiet,
        }
    }
}

/// A language Leteo speaks: its own screens, and — unless the voice has been
/// given one of its own, see [`Settings::voice_language`] — everything Sardi
/// says.
///
/// # One table, two questions
///
/// This list and the one memories are written in are the same twelve languages,
/// and that is enforced here rather than agreed by hand: [`language_choices`]
/// and [`language_for_locale`] both read [`Interface::ALL`]. They were separate
/// tables, and a menu that offers to *store* in a language it will not *speak*
/// is a menu somebody reasonably reads as a promise.
///
/// The two are still not the same kind of thing, and the difference decides what
/// adding a thirteenth costs. [`Settings::language`] is handed to a model, which
/// can write Tagalog whether or not Leteo has heard of it; this one selects
/// between sentences somebody wrote and shipped. So a new entry here is a
/// variant plus every sentence it owes — the compiler names them, in
/// [`crate::i18n`] for the screens and [`crate::sardi`] for the voice — and
/// there is no lookup that can miss a key at runtime and no half-translated
/// screen.
///
/// Latin script throughout, and that is a constraint rather than a preference.
/// The wizard pads these into a column with `{:<10}`, which counts *characters*
/// — so an entry whose glyphs are Wide in East_Asian_Width takes two columns
/// each and shears every row below it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    #[default]
    English,
    Spanish,
    Portuguese,
    French,
    German,
    Italian,
    Catalan,
    Galician,
    Basque,
    Dutch,
    Polish,
    Swedish,
}

impl Interface {
    /// The languages on offer, in the order they are shown.
    ///
    /// English and Spanish first because they are the two Leteo was written in;
    /// the rest follow by how close a neighbour they are to those, which is a
    /// judgement rather than a rule, and better than alphabetical — a menu
    /// sorted by the English spelling of a language is sorted by a name most of
    /// its readers do not use for it.
    pub const ALL: [Self; 12] = [
        Self::English,
        Self::Spanish,
        Self::Portuguese,
        Self::French,
        Self::German,
        Self::Italian,
        Self::Catalan,
        Self::Galician,
        Self::Basque,
        Self::Dutch,
        Self::Polish,
        Self::Swedish,
    ];

    /// The ISO 639-1 code, which is what a machine locale names.
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Spanish => "es",
            Self::Portuguese => "pt",
            Self::French => "fr",
            Self::German => "de",
            Self::Italian => "it",
            Self::Catalan => "ca",
            Self::Galician => "gl",
            Self::Basque => "eu",
            Self::Dutch => "nl",
            Self::Polish => "pl",
            Self::Swedish => "sv",
        }
    }

    /// The language's own name for itself, which is how a menu of languages has
    /// to read: somebody looking for Spanish is looking for "español".
    pub fn as_str(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Spanish => "español",
            Self::Portuguese => "português",
            Self::French => "français",
            Self::German => "Deutsch",
            Self::Italian => "italiano",
            Self::Catalan => "català",
            Self::Galician => "galego",
            Self::Basque => "euskara",
            Self::Dutch => "Nederlands",
            Self::Polish => "polski",
            Self::Swedish => "svenska",
        }
    }

    /// Every spelling of this language a settings file might hold.
    ///
    /// Lower case, and including the accentless form of each name: somebody
    /// typing into JSON on a keyboard without the accent should not be told
    /// their language does not exist. The English name is here because the
    /// documentation is in English, and the Spanish one where Leteo's own two
    /// languages are concerned, because that is who writes this by hand.
    fn spellings(self) -> &'static [&'static str] {
        match self {
            Self::English => &["english", "en", "inglés", "ingles"],
            Self::Spanish => &["español", "espanol", "spanish", "es", "castellano"],
            Self::Portuguese => &["português", "portugues", "portuguese", "pt"],
            Self::French => &["français", "francais", "french", "fr", "francés", "frances"],
            Self::German => &["deutsch", "german", "de", "alemán", "aleman"],
            Self::Italian => &["italiano", "italian", "it"],
            Self::Catalan => &["català", "catala", "catalan", "ca", "catalán"],
            Self::Galician => &["galego", "galician", "gl", "gallego"],
            Self::Basque => &["euskara", "basque", "eu", "euskera", "vasco"],
            Self::Dutch => &["nederlands", "dutch", "nl", "neerlandés", "neerlandes"],
            Self::Polish => &["polski", "polish", "pl", "polaco"],
            Self::Swedish => &["svenska", "swedish", "sv", "sueco"],
        }
    }

    /// Reads a language by name, case and surrounding space forgiven.
    ///
    /// Takes the endonym, the English name and the code, because all three are
    /// things somebody types into a settings file.
    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_lowercase();
        Self::ALL
            .into_iter()
            .find(|language| language.spellings().contains(&value.as_str()))
    }
}

/// Written as the language's own name, which is what the setup screen offered
/// and so what somebody expects to find in the file afterwards.
impl Serialize for Interface {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Read through [`Interface::parse`] rather than through `#[serde(alias)]`.
///
/// The aliases would be a second list of what each language may be called, kept
/// beside the first by hand — and the two would answer differently the moment
/// one of them was added to, with the settings file and the setup screen
/// disagreeing about the same word.
impl<'de> Deserialize<'de> for Interface {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).ok_or_else(|| {
            let known: Vec<&str> = Self::ALL.iter().map(|language| language.as_str()).collect();
            serde::de::Error::custom(format!(
                "{raw:?} is not a language Leteo speaks; it speaks {}",
                known.join(", ")
            ))
        })
    }
}

/// The languages Leteo can name, as locale code and the word for the language
/// in that language.
///
/// Derived from [`Interface::ALL`] rather than written out beside it. One table
/// with three readers now — [`language_for_locale`] looks a machine's locale up
/// in it, [`language_choices`] offers the names on the setup screen, and the
/// interface question offers the same twelve — and the reason it is derived is
/// that those three used to be two lists: what Leteo could *store* in and what
/// it could *speak*, free to drift, and they had.
fn named_languages() -> impl Iterator<Item = (&'static str, &'static str)> {
    Interface::ALL
        .into_iter()
        .map(|language| (language.code(), language.as_str()))
}

/// The language choices the setup offers, in the order they are shown.
///
/// `None` is auto — the language of the conversation. The rest are pinned:
/// that language whatever the conversation is in.
///
/// The machine's own language comes first among the named ones, because it is
/// the likeliest answer and scrolling to find it is work nobody should do. The
/// rest follow in table order, deduplicated against it.
///
/// This used to offer two entries — auto and English — on the reasoning that
/// any list of languages is arbitrary and the settings file takes free text
/// anyway. Both halves were true and the conclusion was still wrong: "arbitrary"
/// argues for choosing the list carefully, not for leaving it at one, and an
/// escape hatch that costs somebody a JSON file is not an answer to a question
/// they were just asked on screen.
pub fn language_choices(system: Option<&str>) -> Vec<Option<String>> {
    let system = system
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let mut choices = vec![None];
    choices.extend(system.clone().map(Some));
    choices.extend(
        named_languages()
            .map(|(_, name)| name.to_owned())
            .filter(|name| system.as_deref() != Some(name.as_str()))
            .map(Some),
    );
    choices
}

/// The language this machine is set to, as a word a model understands.
///
/// The environment first, because that is where every shell and every CI runner
/// agrees to put it — and then the operating system, because **Windows does not
/// put it there at all**. `LANG`, `LC_ALL`, `LC_MESSAGES` and `LANGUAGE` are
/// POSIX conventions; on Windows all four are simply unset, so reading only the
/// environment answered `None` on the one platform where it had nothing else to
/// fall back to. The wizard then offered no suggestion to precisely the person
/// whose machine already knew the answer: this one reports `es-ES`.
///
/// `LETEO_SYSTEM_LANGUAGE` is checked first rather than last. It is the only
/// way to state the answer where detection is wrong or absent, and an override
/// that loses to whatever `LANG` happens to hold is not an override.
///
/// Unknown stays `None`, which costs one menu entry and never a wrong one.
pub fn system_language() -> Option<String> {
    let raw = std::env::var("LETEO_SYSTEM_LANGUAGE")
        .ok()
        .or_else(|| {
            ["LANG", "LC_ALL", "LC_MESSAGES", "LANGUAGE"]
                .iter()
                .find_map(|name| std::env::var(name).ok())
        })
        .or_else(platform_locale)?;
    language_for_locale(&raw)
}

/// The language a locale string names, or `None` for one this does not know.
///
/// Split out from the lookup so it can be tested without writing to the
/// process environment, which every other test would then be racing.
fn language_for_locale(raw: &str) -> Option<String> {
    let code = raw.split(['_', '-', '.', ':']).next()?.to_ascii_lowercase();
    named_languages()
        .find(|(known, _)| *known == code)
        .map(|(_, name)| name.to_owned())
}

/// What the operating system says, where the environment says nothing.
///
/// Only Windows needs this, and only because it is the one platform with no
/// environment convention to read. `GetUserDefaultLocaleName` gives a BCP-47
/// name — `es-ES` — which the parser above already understands, so nothing else
/// has to learn a second shape.
#[cfg(windows)]
fn platform_locale() -> Option<String> {
    // `LOCALE_NAME_MAX_LENGTH`, which the call will not exceed.
    let mut buffer = [0u16; 85];
    // Returns the length written *including* the terminating NUL, or 0 on
    // failure — so one is "just the NUL", which is no answer either.
    let written = unsafe {
        windows_sys::Win32::Globalization::GetUserDefaultLocaleName(
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    if written <= 1 {
        return None;
    }
    String::from_utf16(&buffer[..written as usize - 1]).ok()
}

#[cfg(not(windows))]
fn platform_locale() -> Option<String> {
    None
}

/// How many memories a session opens with.
///
/// The opening block is an index of titles, and how long that index is, is the
/// one number that decides what a session costs before anybody has asked
/// anything. It was fifty, chosen for shape rather than for size.
///
/// Measured on a real store, over its 586 distinct questions: for each one,
/// the memory that best answers it among those written *before* it was found,
/// and then asked whether the opening block would already have named it.
///
/// ```text
///   memories   already named   bytes
///         10          32.2%    2,490
///         20          44.5%    4,980
///         30          50.4%    7,470
///         50          55.9%   12,450
///         80          61.5%   19,920
/// ```
///
/// The first twenty buy 4.9 points a kilobyte and the last thirty buy 0.75, so
/// the knee is around twenty to thirty and `Full` sits past it deliberately:
/// what an agent is not told about, it does not know to ask for.
///
/// Three named sizes rather than a number, because the number that matters is
/// what it buys, and that is what the names are for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextSize {
    /// Twenty: 44.5% for 5 KB, for a small context window or a long session.
    Slim,
    /// Fifty. What a session opens with unless somebody says otherwise.
    #[default]
    Full,
    /// Eighty: 61.5% for 20 KB, when the store matters more than the budget.
    Deep,
}

impl ContextSize {
    pub fn memories(self) -> usize {
        match self {
            Self::Slim => 20,
            Self::Full => 50,
            Self::Deep => 80,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Slim => "slim",
            Self::Full => "full",
            Self::Deep => "deep",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "slim" => Some(Self::Slim),
            "full" => Some(Self::Full),
            "deep" => Some(Self::Deep),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Settings {
    pub voice: Voice,
    /// The language memories are written in.
    ///
    /// `None` means the language of the conversation, which is the right
    /// default and not the behaviour that was happening. Memories are written
    /// by an agent, and an agent left to itself writes English whatever it was
    /// asked in — a real store of 3,550 held about 90% English notes against
    /// 59% Spanish questions. That was treated for a long time as a fact to
    /// work around in the search, and it is a defect: a memory written in a
    /// language its reader did not use is harder to find and harder to read,
    /// and no amount of cleverness downstream undoes either.
    ///
    /// Set it to pin one language regardless — for somebody who works in
    /// Spanish but pastes English stack traces all day, "the language of the
    /// conversation" is ambiguous where "Spanish" is not.
    ///
    /// Free text rather than a code, because it is handed to a model, not
    /// parsed: `español`, `Spanish`, `português do Brasil` all work, and a list
    /// of enum variants would be a list of languages Leteo refuses to remember
    /// in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The language Leteo's own screens and Sardi's lines are written in.
    ///
    /// `None` follows the machine, which is the right default and the reason
    /// this is an `Option` rather than an [`Interface`] with a default variant:
    /// "never chose" and "chose English" have to stay distinguishable, or a
    /// person on a Spanish machine who deliberately wants the English interface
    /// gets moved back every time detection runs.
    ///
    /// Separate from [`Settings::language`] because they answer different
    /// questions, and conflating them is a mistake worth naming: somebody who
    /// works in Spanish may well want their memories in English so their
    /// searches match the English identifiers and stack traces they paste all
    /// day — and somebody whose team stores memories in English still deserves
    /// to be spoken to in their own language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface: Option<Interface>,
    /// The language Sardi speaks, when it is not the one Leteo speaks.
    ///
    /// `None` means "whatever Leteo is speaking", and that is a live answer
    /// rather than an unset one: it keeps following [`Settings::interface`] as
    /// that changes, including as the machine changes underneath it.
    ///
    /// Split from [`Settings::interface`] because the two are not read in the
    /// same place. Leteo's own screens are a program somebody opens; Sardi's
    /// lines are emitted by lifecycle hooks *into an agent's conversation* —
    /// alongside whatever language that conversation is being held in. Somebody
    /// working with an agent in English, on a Spanish machine, has a real
    /// reason to want the panels in Spanish and the cat quiet in English, and
    /// with one field they could have neither.
    ///
    /// The cost is real and is the reason this defaults to following: two
    /// fields is two ways to end up with half a screen in each language. So the
    /// default is not a language at all — it is the other setting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_language: Option<Interface>,
    /// How many memories a session opens with. See [`ContextSize`].
    ///
    /// `None` means [`ContextSize::Full`], and stays distinguishable from
    /// having chosen `full` for the same reason the languages do: a default
    /// that moves should move for somebody who never chose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_size: Option<ContextSize>,
}

/// The same fields, read as whatever the file happens to hold.
///
/// The step that makes a bad value cost one setting instead of all of them.
/// Serde stops at the first field it cannot read and fails the whole struct,
/// and this file is one a person is *expected* to open — it exists so that a
/// memory tool which has started talking too much can be answered by editing a
/// file rather than by learning a subcommand.
///
/// So one typo took every other answer with it, and said nothing: a store set
/// to remember in Spanish went back to the default of "whatever the model
/// feels like", which is the failure nobody connects to what they typed,
/// because the sign of it is memories written in the wrong language three
/// weeks later.
#[derive(Deserialize)]
#[serde(default)]
struct RawSettings {
    voice: serde_json::Value,
    language: serde_json::Value,
    interface: serde_json::Value,
    voice_language: serde_json::Value,
    context_size: serde_json::Value,
}

impl Default for RawSettings {
    fn default() -> Self {
        Self {
            voice: serde_json::Value::Null,
            language: serde_json::Value::Null,
            interface: serde_json::Value::Null,
            voice_language: serde_json::Value::Null,
            context_size: serde_json::Value::Null,
        }
    }
}

impl<'de> Deserialize<'de> for Settings {
    /// Field by field, each falling back to its own default.
    ///
    /// A field that is missing, null, or unreadable all mean the same thing —
    /// nobody set this one — which is the reading every field here already had
    /// for absence. What changes is that it stays that field's business.
    ///
    /// A file that is not JSON at all is still all defaults, and deliberately:
    /// hooks read it on every event, so a half-written file has to be survived
    /// rather than reported. See [`load`].
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawSettings::deserialize(deserializer)?;
        Ok(Self {
            voice: serde_json::from_value(raw.voice).unwrap_or_default(),
            language: serde_json::from_value(raw.language).unwrap_or_default(),
            interface: serde_json::from_value(raw.interface).unwrap_or_default(),
            voice_language: serde_json::from_value(raw.voice_language).unwrap_or_default(),
            context_size: serde_json::from_value(raw.context_size).unwrap_or_default(),
        })
    }
}

impl Settings {
    pub fn context_size(&self) -> ContextSize {
        self.context_size.unwrap_or_default()
    }

    /// The language to speak, resolved: what was chosen, else what the machine
    /// is set to, else English.
    ///
    /// English last rather than first. It is the only language guaranteed to
    /// have every sentence, so it is the floor — but reaching for it before
    /// asking the machine is how an interface ends up in English on a computer
    /// that has been telling everyone it is Spanish since it was unboxed.
    pub fn interface(&self) -> Interface {
        self.interface_or(system_language().as_deref())
    }

    /// The same answer, from a language handed in rather than detected.
    ///
    /// Split out so it can be tested without writing to the process
    /// environment, which every other test would then be racing — the same
    /// reason [`language_for_locale`] is separate from [`system_language`].
    fn interface_or(&self, system: Option<&str>) -> Interface {
        self.interface
            .or_else(|| system.and_then(Interface::parse))
            .unwrap_or_default()
    }

    /// The language Sardi speaks, resolved: what was chosen for the voice, else
    /// whatever Leteo itself is speaking.
    ///
    /// Every line the character says goes through this rather than through
    /// [`Settings::interface`] — the greeting a hook writes into an agent's
    /// conversation and the one on the dashboard header alike. Sending them to
    /// different settings by where they are painted would be a rule nobody
    /// could state, and the first surprise would be the header disagreeing with
    /// the hook that had just run.
    pub fn voice_language(&self) -> Interface {
        self.voice_language.unwrap_or_else(|| self.interface())
    }

    /// The sentence an agent is given about which language to save in.
    ///
    /// Always says something. Left implicit, an agent defaults to English, so
    /// the case that needs stating out loud is the ordinary one.
    pub fn language_directive(&self) -> String {
        match self
            .language
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(language) => {
                format!(
                    "Write and search memories in {language}, whatever language this conversation is in."
                )
            }
            // One line, deliberately, and `concat!` is one way to get there.
            //
            // The reason written here before was that a `\` continuation keeps
            // the next line's indentation inside the string. It does the
            // opposite: `\` before a newline eats the newline *and* the leading
            // whitespace, which is exactly what makes it the usual idiom for
            // this. What does keep the indentation is a plain multi-line
            // literal with no `\` at all — that is the trap, and it had already
            // put a run of thirty spaces into the middle of a wizard sentence.
            None => concat!(
                "Write each memory in the language the user is writing in, not ",
                "in English by default; keep identifiers, paths and error ",
                "strings as they are. Search in that language too, and try ",
                "another if nothing comes back.",
            )
            .to_owned(),
        }
    }
}

/// Where the file lives for a given data directory.
///
/// Beside the database rather than in a fixed home directory, so a second data
/// directory — a test's, a second machine's, one pointed at by
/// `LETEO_DATA_DIR` — carries its own answer instead of borrowing one.
pub fn path_in(data_dir: impl AsRef<Path>) -> PathBuf {
    data_dir.as_ref().join("settings.json")
}

/// The settings for a data directory, or the defaults if they cannot be read.
pub fn load(data_dir: impl AsRef<Path>) -> Settings {
    std::fs::read_to_string(path_in(data_dir))
        .ok()
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default()
}

/// Which answers in the settings file are being ignored, and why.
///
/// Each field falls back to its own default when it cannot be read — see the
/// `Deserialize` above, which is what stops one typo taking the rest of the
/// file with it. That is the right behaviour for a hook reading this on every
/// event, and it leaves a person with no way to find out: `"context_size":
/// "slimm"` is answered with the default size and not a word, and the sign of
/// it is a context that is the wrong length weeks later.
///
/// So the same reading is done once more, out loud, for anything that asks.
/// A file that is not JSON at all is one entry rather than five.
pub fn ignored(data_dir: impl AsRef<Path>) -> Vec<String> {
    let path = path_in(data_dir);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(raw) = serde_json::from_str::<serde_json::Value>(&body) else {
        return vec!["the file is not JSON".to_owned()];
    };
    let Some(fields) = raw.as_object() else {
        return vec!["the file is not a JSON object".to_owned()];
    };
    let mut ignored = Vec::new();
    let mut check = |name: &str, parses: bool| {
        if let Some(value) = fields.get(name).filter(|value| !value.is_null())
            && !parses
        {
            ignored.push(format!("{name} is {value}"));
        }
    };
    check(
        "voice",
        fields
            .get("voice")
            .is_none_or(|value| serde_json::from_value::<Voice>(value.clone()).is_ok()),
    );
    check(
        "interface",
        fields
            .get("interface")
            .is_none_or(|value| serde_json::from_value::<Interface>(value.clone()).is_ok()),
    );
    check(
        "voice_language",
        fields
            .get("voice_language")
            .is_none_or(|value| serde_json::from_value::<Interface>(value.clone()).is_ok()),
    );
    check(
        "context_size",
        fields
            .get("context_size")
            .is_none_or(|value| serde_json::from_value::<ContextSize>(value.clone()).is_ok()),
    );
    check(
        "language",
        fields.get("language").is_none_or(|value| value.is_string()),
    );
    // And a key that is not one of the five, which is the same typo one letter
    // earlier: `contextsize` without its underscore is read past exactly as
    // quietly as `slimm` was. `save` writes only these names, so anything else
    // in the file was typed by a person.
    let known = setting_names();
    for name in fields.keys() {
        if !known.iter().any(|known| known == name) {
            ignored.push(format!("{name} is not a setting"));
        }
    }
    ignored
}

/// The names `save` writes, taken from the struct rather than typed again.
///
/// This list used to be spelled out here as a literal, which made it the second
/// copy of something the struct above already says. A sixth setting added to
/// [`Settings`] would have been read fine, written fine, and then reported as
/// "not a setting" by the copy nobody remembered to grow — the failure this
/// function exists to prevent, produced by the function itself.
///
/// The literal below is the same names once more, but the compiler will not let
/// it fall behind: a field added to [`Settings`] stops this from building.
fn setting_names() -> Vec<String> {
    let every = Settings {
        voice: Voice::default(),
        language: Some(String::new()),
        interface: Some(Interface::default()),
        voice_language: Some(Interface::default()),
        context_size: Some(ContextSize::default()),
    };
    serde_json::to_value(&every)
        .ok()
        .and_then(|value| match value {
            serde_json::Value::Object(fields) => Some(fields.keys().cloned().collect()),
            _ => None,
        })
        .unwrap_or_default()
}

pub fn load_beside(database_path: &Path) -> Settings {
    database_path.parent().map(load).unwrap_or_default()
}

/// Writes the settings, creating the data directory if it is not there yet.
///
/// This one does report failure. It runs when somebody has just answered a
/// question, and silently discarding their answer would be worse than saying
/// the disk is full.
pub fn save(data_dir: impl AsRef<Path>, settings: &Settings) -> Result<()> {
    let path = path_in(&data_dir);
    let mut body = serde_json::to_string_pretty(settings).context("serialize settings")?;
    body.push('\n');
    // A hook reads this file on every event, so a half-written one would be
    // read while it is half written. `load` treats unreadable settings as the
    // default, which would silence Sardi for exactly as long as the truncation
    // lasted and give no sign of why.
    crate::files::replace(&path, body.as_bytes())
        .with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Every setting has to be one `ignored` will speak up about. A field can
    /// be added to [`Settings`], read, written, and never checked here, and the
    /// only sign of it is somebody's answer quietly not applying — which is the
    /// whole thing `ignored` was written to end. So rather than trust that the
    /// five `check` calls kept up, this drives every name the struct serialises
    /// through a value that cannot parse and insists it is named back.
    ///
    /// The value is an empty list because it is the one shape none of the five
    /// can take: a string would be a perfectly good `language`, which is free
    /// text on purpose, and the first version of this test called that silence
    /// a defect.
    #[test]
    fn every_setting_there_is_gets_reported_when_it_cannot_be_read() {
        let names = setting_names();
        assert_eq!(names.len(), 5, "{names:?}");
        for name in &names {
            let temp = TempDir::new().unwrap();
            let body = format!("{{{:?}: []}}", name);
            std::fs::write(path_in(temp.path()), &body).unwrap();
            let said = ignored(temp.path());
            assert!(
                said.iter().any(|line| line.starts_with(name)),
                "{name} was ignored in silence: {said:?}"
            );
        }
    }

    /// `language` is free text, so it is the one name the loop above cannot
    /// break with a string — and the one whose own check has to be a string
    /// test rather than a parse.
    #[test]
    fn a_setting_that_is_the_wrong_shape_is_reported_and_a_stray_key_is_named() {
        let temp = TempDir::new().unwrap();
        std::fs::write(
            path_in(temp.path()),
            r#"{"language": 7, "contextsize": "slim", "context_size": "slimm"}"#,
        )
        .unwrap();
        let said = ignored(temp.path());
        assert!(said.iter().any(|line| line == "language is 7"), "{said:?}");
        assert!(
            said.iter()
                .any(|line| line == "contextsize is not a setting"),
            "the typo one letter earlier is the one that reads past: {said:?}"
        );
        assert!(
            said.iter().any(|line| line == "context_size is \"slimm\""),
            "{said:?}"
        );

        std::fs::write(path_in(temp.path()), "not json at all").unwrap();
        assert_eq!(ignored(temp.path()), vec!["the file is not JSON"]);
        std::fs::write(path_in(temp.path()), "[1, 2]").unwrap();
        assert_eq!(ignored(temp.path()), vec!["the file is not a JSON object"]);

        assert!(ignored(TempDir::new().unwrap().path()).is_empty());
    }

    #[test]
    fn the_reminder_survives_turning_the_reports_off() {
        // The whole reason there are three levels rather than a boolean: the
        // middle one has to keep the line that makes an agent save anything.
        assert!(Voice::All.reports() && Voice::All.reminders());
        assert!(!Voice::Reminders.reports() && Voice::Reminders.reminders());
        assert!(!Voice::Quiet.reports() && !Voice::Quiet.reminders());
    }

    #[test]
    fn an_unset_preference_is_the_loudest_one() {
        // Installing Leteo and hearing nothing would read as a broken install.
        assert_eq!(Voice::default(), Voice::All);
        assert_eq!(Settings::default().voice, Voice::All);
    }

    #[test]
    fn a_level_survives_being_written_and_read_back() {
        let temp = TempDir::new().unwrap();
        for voice in Voice::ALL {
            save(
                temp.path(),
                &Settings {
                    voice,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(load(temp.path()).voice, voice, "{voice:?}");
        }
        let body = std::fs::read_to_string(path_in(temp.path())).unwrap();
        assert!(body.contains("\"voice\": \"quiet\""), "{body}");
    }

    #[test]
    fn nothing_a_person_can_do_to_the_file_stops_a_hook() {
        // Each of these is a real state a settings file reaches: never created,
        // half-written by a killed process, edited to a level that does not
        // exist, or emptied. All of them have to mean the defaults.
        let temp = TempDir::new().unwrap();
        assert_eq!(load(temp.path()), Settings::default(), "absent");
        for broken in ["", "   ", "{", "{\"voice\":\"loud\"}", "null", "[]"] {
            std::fs::write(path_in(temp.path()), broken).unwrap();
            assert_eq!(load(temp.path()), Settings::default(), "{broken:?}");
        }
    }

    #[test]
    fn a_level_can_be_named_the_way_a_person_would_type_it() {
        assert_eq!(Voice::parse(" Quiet "), Some(Voice::Quiet));
        assert_eq!(Voice::parse("REMINDERS"), Some(Voice::Reminders));
        assert_eq!(Voice::parse("silent"), None);
        for voice in Voice::ALL {
            assert_eq!(Voice::parse(voice.as_str()), Some(voice));
        }
    }

    #[test]
    fn one_unreadable_field_costs_that_field_and_leaves_the_rest_standing() {
        // This file is meant to be opened by hand — that is why it is a file
        // and not a row in the database. So it has to survive being typed in.
        //
        // Serde stops at the first field it cannot read and fails the whole
        // struct, and the caller reads that as "no settings at all". One typo
        // therefore silenced three answers nobody had touched, and the one that
        // hurt was the memory language: back to the default, and the sign of it
        // is memories written in the wrong language three weeks later.
        let temp = TempDir::new().unwrap();
        let written = |body: &str| {
            std::fs::write(path_in(temp.path()), body).unwrap();
            load(temp.path())
        };

        let kept = written(
            r#"{"voice":"quiet","language":"español","interface":"español",
                "voice_language":"klingon"}"#,
        );
        assert_eq!(kept.voice, Voice::Quiet, "a typo cost the voice");
        assert_eq!(kept.language.as_deref(), Some("español"));
        assert_eq!(kept.interface, Some(Interface::Spanish));
        assert_eq!(
            kept.voice_language, None,
            "the field that could not be read is the one that goes"
        );

        let kept = written(r#"{"voice":7,"language":"español","interface":"español"}"#);
        assert_eq!(kept.voice, Voice::All, "an unreadable level is no level");
        assert_eq!(kept.language.as_deref(), Some("español"));
        assert_eq!(kept.interface, Some(Interface::Spanish));

        let kept = written(r#"{"voice":"quiet","language":[],"interface":"nope"}"#);
        assert_eq!(kept.voice, Voice::Quiet);
        assert_eq!(kept.language, None);
        assert_eq!(kept.interface, None);

        // A file that is not JSON at all is still all defaults, and that one is
        // deliberate: a hook reads this on every event, and a half-written file
        // has to be survived rather than reported.
        assert_eq!(written("{\"voice\": \"qui"), Settings::default());
        // As is a file that holds something else entirely.
        assert_eq!(written("[1, 2, 3]"), Settings::default());
    }

    #[test]
    fn the_voice_follows_leteos_language_until_it_is_given_one() {
        // Unset is not "English", it is "whatever the other setting says" —
        // and it has to keep meaning that as the other setting changes, or the
        // default would be a language somebody was pinned to on the day they
        // installed.
        let mut settings = Settings {
            interface: Some(Interface::Spanish),
            ..Settings::default()
        };
        assert_eq!(settings.voice_language(), Interface::Spanish);
        settings.interface = Some(Interface::Basque);
        assert_eq!(settings.voice_language(), Interface::Basque);

        settings.voice_language = Some(Interface::English);
        assert_eq!(settings.voice_language(), Interface::English);
        assert_eq!(
            settings.interface(),
            Interface::Basque,
            "the voice's language must not answer for the screens"
        );
    }

    #[test]
    fn a_voice_with_no_language_of_its_own_is_written_as_having_none() {
        // The two answers a file has to keep apart: "follow Leteo", which is
        // the absence of the key, and a pinned language that happens to equal
        // what Leteo speaks today. Writing the resolved value would collapse
        // them and freeze the first into the second.
        let temp = TempDir::new().unwrap();
        save(
            temp.path(),
            &Settings {
                interface: Some(Interface::Spanish),
                ..Settings::default()
            },
        )
        .unwrap();
        let written = std::fs::read_to_string(path_in(temp.path())).unwrap();
        assert!(
            !written.contains("voice_language"),
            "a voice that follows must not be written down as pinned: {written}"
        );

        save(
            temp.path(),
            &Settings {
                interface: Some(Interface::Spanish),
                voice_language: Some(Interface::English),
                ..Settings::default()
            },
        )
        .unwrap();
        let read = load(temp.path());
        assert_eq!(read.voice_language, Some(Interface::English));
        assert_eq!(read.interface, Some(Interface::Spanish));
    }

    #[test]
    fn settings_belong_to_the_directory_their_database_is_in() {
        let temp = TempDir::new().unwrap();
        save(
            temp.path(),
            &Settings {
                language: None,
                voice: Voice::Quiet,
                interface: None,
                voice_language: None,
                context_size: None,
            },
        )
        .unwrap();
        let database = temp.path().join("leteo.db");
        assert_eq!(load_beside(&database).voice, Voice::Quiet);
        let other = TempDir::new().unwrap();
        assert_eq!(
            load_beside(&other.path().join("leteo.db")).voice,
            Voice::All
        );
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn a_locale_name_becomes_a_word_a_model_understands() {
        // The three shapes this has to read: POSIX with an encoding, POSIX
        // bare, and the BCP-47 name Windows hands back.
        for raw in ["es_ES.UTF-8", "es_ES", "es-ES", "es"] {
            assert_eq!(
                language_for_locale(raw).as_deref(),
                Some("español"),
                "{raw}"
            );
        }
        assert_eq!(language_for_locale("en_GB").as_deref(), Some("English"));
        assert_eq!(language_for_locale("pt-BR").as_deref(), Some("português"));

        // A language with no entry offers nothing rather than something wrong.
        // It costs one menu line; the settings file still takes free text.
        assert_eq!(language_for_locale("ja_JP"), None);
        assert_eq!(language_for_locale(""), None);
        assert_eq!(language_for_locale("C"), None);
    }

    /// Windows answers, where the environment never does.
    ///
    /// `LANG`, `LC_ALL`, `LC_MESSAGES` and `LANGUAGE` are POSIX conventions and
    /// Windows sets none of them, so reading only the environment returned
    /// `None` on the one platform with nothing to fall back to — and the setup
    /// wizard suggested nothing to somebody whose machine already knew.
    ///
    /// Asserts the *shape*, not the value: this has to pass on a machine set to
    /// any language, including one with no entry in the table above.
    /// And end to end: what the wizard is actually handed on this machine.
    #[cfg(windows)]
    #[test]
    fn the_wizard_is_offered_the_language_this_machine_is_set_to() {
        // Only meaningful where the environment is silent, which is the whole
        // point on Windows — if a shell has set `LANG`, that path already
        // worked and this proves nothing.
        if [
            "LANG",
            "LC_ALL",
            "LC_MESSAGES",
            "LANGUAGE",
            "LETEO_SYSTEM_LANGUAGE",
        ]
        .iter()
        .any(|name| std::env::var(name).is_ok())
        {
            return;
        }
        let detected = system_language();
        let choices = language_choices(detected.as_deref());
        // Auto and every named language are always there. A machine set to one
        // this does not recognise adds nothing; one it does recognise adds no
        // entry either — it moves that language to the front.
        assert_eq!(
            choices.len(),
            named_languages().count() + 1,
            "no language may be offered twice or dropped: {choices:?}"
        );
        if let Some(detected) = detected {
            assert_eq!(
                choices.get(1),
                Some(&Some(detected.clone())),
                "{detected} was detected and then not offered first: {choices:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_is_asked_because_it_puts_the_answer_nowhere_else() {
        let locale = platform_locale().expect("Windows always has a user locale");
        let (language, _) = locale.split_once('-').unwrap_or((locale.as_str(), ""));
        assert!(
            language.len() >= 2 && language.chars().all(|c| c.is_ascii_alphabetic()),
            "expected a BCP-47 name such as es-ES, got {locale:?}"
        );

        // And that the lookup actually reaches it. Asserting on
        // `platform_locale` alone proved only that the call works — deleting
        // the `.or_else` that uses it left this test green, which is the same
        // "nothing noticed" this whole file is about.
        if [
            "LANG",
            "LC_ALL",
            "LC_MESSAGES",
            "LANGUAGE",
            "LETEO_SYSTEM_LANGUAGE",
        ]
        .iter()
        .any(|name| std::env::var(name).is_ok())
        {
            return;
        }
        assert_eq!(
            system_language(),
            language_for_locale(&locale),
            "with the environment silent, the machine's own answer is the answer"
        );
    }
}

#[cfg(test)]
mod context_size_tests {
    use super::*;

    /// The three sizes are the measured ones, and unset is the middle.
    ///
    /// Measured over the 586 distinct questions of a real store: for each, the
    /// memory that best answers it among those written before it, and whether
    /// the opening block would have named it. 20 buys 44.5%, 50 buys 55.9%,
    /// 80 buys 61.5%. Written out rather than read from the enum, because the
    /// numbers *are* the decision.
    #[test]
    fn each_size_names_the_measured_number_of_memories() {
        assert_eq!(ContextSize::Slim.memories(), 20);
        assert_eq!(ContextSize::Full.memories(), 50);
        assert_eq!(ContextSize::Deep.memories(), 80);
        assert_eq!(ContextSize::default(), ContextSize::Full);
        assert_eq!(Settings::default().context_size().memories(), 50);
    }

    #[test]
    fn a_size_is_read_however_it_was_written() {
        for (written, expected) in [
            ("slim", ContextSize::Slim),
            ("FULL", ContextSize::Full),
            ("  Deep  ", ContextSize::Deep),
        ] {
            assert_eq!(ContextSize::parse(written), Some(expected), "{written}");
        }
        // And a word that is not one of the three is refused rather than
        // guessed at: the caller turns that into an error naming the three.
        assert_eq!(ContextSize::parse("medium"), None);
        assert_eq!(ContextSize::parse(""), None);
    }

    #[test]
    fn an_unreadable_size_does_not_take_the_other_settings_with_it() {
        let settings: Settings = serde_json::from_str(
            r#"{"language":"español","context_size":"enorme","voice":"quiet"}"#,
        )
        .expect("a settings file is read field by field");
        assert_eq!(settings.language.as_deref(), Some("español"));
        assert_eq!(settings.voice, Voice::Quiet);
        assert_eq!(
            settings.context_size().memories(),
            50,
            "an unreadable size is nobody having chosen one"
        );
    }
}
