//! The second stemmer: a Snowball stemmer for the language memories are written in.
//!
//! FTS5 takes its tokenizer from a fixed list, so a stemmer written in Rust
//! cannot be named in `tokenize =`. The words are stemmed *before* they reach an
//! index instead, by the two SQL functions registered here, and the index that
//! holds them (`observations_stemmed`) tokenises plain `unicode61`. The query side
//! applies the same function, so a word and the stem it was indexed under meet.
//!
//! Which languages have a stemmer is the one table in [`algorithm`]; every other
//! place asks it.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;

use crate::settings::Interface;

/// The Snowball algorithm for a language, where this build has one.
///
/// Two sources are behind it, and the split is measured rather than tidy; the
/// table in [`algorithm`] says which language comes from which and why.
pub enum Algorithm {
    /// One of the eight `rust-stemmers` carries.
    Rust(rust_stemmers::Algorithm),
    /// One of the three only `snowball_stemmers_rs` carries.
    Snowball(snowball_stemmers_rs::Algorithm),
}

/// A stemmer over either source, so `stem_text` does not branch per word.
enum Stemmer {
    Rust(rust_stemmers::Stemmer),
    Snowball(snowball_stemmers_rs::Stemmer),
}

impl Stemmer {
    fn create(algorithm: Algorithm) -> Self {
        match algorithm {
            Algorithm::Rust(algorithm) => Self::Rust(rust_stemmers::Stemmer::create(algorithm)),
            Algorithm::Snowball(algorithm) => {
                Self::Snowball(snowball_stemmers_rs::Stemmer::create(algorithm))
            }
        }
    }

    fn stem<'a>(&self, word: &'a str) -> Cow<'a, str> {
        match self {
            Self::Rust(stemmer) => stemmer.stem(word),
            Self::Snowball(stemmer) => stemmer.stem(word),
        }
    }
}

/// The Snowball algorithm for a language, where this build has one.
///
/// English is absent on purpose: `porter` already indexes every memory, and a
/// row's English stems are those. Galician is absent because it has no Snowball
/// algorithm at all, and borrowing one was measured and declined; the set that
/// measured it records why. The other eleven languages the
/// setting offers each have an arm: Spanish, Portuguese, French, German,
/// Italian, Romanian, Dutch and Swedish come from `rust-stemmers`, and Catalan,
/// Basque and Polish, which it does not carry, come from
/// `snowball_stemmers_rs`. A row in a language with no arm here is recorded
/// under it and gets `porter` alone.
///
/// The two sources are not interchangeable. `snowball_stemmers_rs` also carries
/// the eight, but it disagrees with `rust-stemmers` on words they already index
/// — Dutch alone differs on 102,728 words of this repository's own vocabulary —
/// so the eight keep the source their floors were measured against.
///
/// A language added here needs a set in `tools/memory-bench/inflection_sets.py`
/// and a floor for it: a stemmer that no query can tell from `porter` is not
/// shipping anything.
pub fn algorithm(language: Interface) -> Option<Algorithm> {
    Some(match language {
        Interface::Spanish => Algorithm::Rust(rust_stemmers::Algorithm::Spanish),
        Interface::Portuguese => Algorithm::Rust(rust_stemmers::Algorithm::Portuguese),
        Interface::French => Algorithm::Rust(rust_stemmers::Algorithm::French),
        Interface::German => Algorithm::Rust(rust_stemmers::Algorithm::German),
        Interface::Italian => Algorithm::Rust(rust_stemmers::Algorithm::Italian),
        Interface::Romanian => Algorithm::Rust(rust_stemmers::Algorithm::Romanian),
        Interface::Dutch => Algorithm::Rust(rust_stemmers::Algorithm::Dutch),
        Interface::Swedish => Algorithm::Rust(rust_stemmers::Algorithm::Swedish),
        Interface::Catalan => Algorithm::Snowball(snowball_stemmers_rs::Algorithm::Catalan),
        Interface::Basque => Algorithm::Snowball(snowball_stemmers_rs::Algorithm::Basque),
        Interface::Polish => Algorithm::Snowball(snowball_stemmers_rs::Algorithm::Polish),
        _ => return None,
    })
}

/// The language a settings value names, if it names one Leteo speaks.
///
/// `None` is the setting's "the language of the conversation", which says
/// nothing about what was written. A value Leteo has never heard of says as
/// little: the setting is free text handed to a model, so it can name a language
/// no stemmer exists for, and detection decides in that case as it does for none.
pub fn named_language(setting: Option<&str>) -> Option<Interface> {
    setting.and_then(Interface::parse)
}

/// The language a settings value names, or English when it names none.
///
/// This is the language a connection answers `leteo_stem_language()` with, which
/// is no longer what a row is recorded under when the setting names none: see
/// [`detect`].
pub fn memory_language(setting: Option<&str>) -> Interface {
    named_language(setting).unwrap_or_default()
}

/// The function words each candidate language is recognised by.
///
/// The one table detection, `doctor` and the tests all read. A word that appears
/// in the lists of two languages — `de`, `que`, `por`, `con` — is kept here
/// because it is a stop word in both, and is dropped when the table is indexed:
/// it cannot say which of the two a text is in, and counting it for both would
/// let a Spanish sentence full of `por` and `que` score as much for Portuguese
/// or French as for Spanish.
/// The lists are short on purpose. They are the commonest function words, which
/// a text of a sentence or two carries several of, and not a vocabulary.
///
/// The twelve are the languages [`algorithm`] has an arm for and English, which
/// has none and is the answer when nothing else is clear.
pub const STOP_WORDS: [(Interface, &[&str]); 12] = [
    (
        Interface::English,
        &[
            "the", "and", "of", "to", "that", "it", "for", "with", "was", "are", "this", "be",
            "not", "by", "or", "from", "have", "has", "which", "when", "but", "will", "can",
            "would", "we", "you", "they", "there", "been", "were", "what", "because", "then",
            "than", "into", "also", "is",
        ],
    ),
    (
        Interface::Spanish,
        &[
            "el", "los", "las", "una", "unos", "unas", "que", "y", "en", "es", "por", "con",
            "para", "se", "su", "sus", "lo", "como", "más", "pero", "esto", "este", "son", "fue",
            "han", "sin", "sobre", "entre", "cuando", "también", "porque", "hay", "muy", "ya",
            "está", "están", "del", "al",
        ],
    ),
    (
        Interface::Portuguese,
        &[
            "os", "uma", "uns", "umas", "do", "da", "dos", "das", "em", "no", "na", "nos", "nas",
            "não", "com", "para", "por", "mais", "mas", "como", "foi", "são", "ao", "aos",
            "também", "muito", "já", "seu", "sua", "pelo", "pela", "isso", "esse", "essa", "sem",
            "quando", "porque", "você",
        ],
    ),
    (
        Interface::French,
        &[
            "le", "les", "une", "des", "du", "et", "est", "dans", "pour", "pas", "sur", "aux",
            "avec", "cette", "ces", "il", "elle", "ils", "nous", "vous", "sont", "été", "être",
            "mais", "où", "par", "plus", "ne", "ses", "leur", "aussi", "très", "fait", "peut",
            "ont", "était", "qui", "que",
        ],
    ),
    (
        Interface::German,
        &[
            "der", "die", "das", "dem", "des", "ein", "eine", "einer", "einen", "und", "ist",
            "nicht", "von", "zu", "mit", "auf", "für", "im", "dass", "sich", "auch", "als", "wie",
            "bei", "nach", "aus", "wird", "werden", "wurde", "sind", "hat", "haben", "kann",
            "oder", "aber", "nur", "noch", "wenn", "diese", "ich", "wir",
        ],
    ),
    (
        Interface::Italian,
        &[
            "il", "gli", "uno", "della", "dei", "delle", "di", "che", "è", "per", "sono", "più",
            "ma", "anche", "questo", "questa", "questi", "nel", "nella", "alla", "hanno", "essere",
            "stato", "perché", "tra", "fra", "loro", "molto", "una", "con", "non", "come",
            "quando",
        ],
    ),
    (
        Interface::Romanian,
        &[
            "și", "în", "cu", "pentru", "care", "este", "sunt", "nu", "mai", "din", "sau", "dar",
            "că", "să", "fost", "acest", "această", "aceste", "foarte", "după", "între", "prin",
            "lor", "fi", "când", "dacă", "pe", "la", "un", "o", "se", "cum",
        ],
    ),
    (
        Interface::Dutch,
        &[
            "het", "een", "van", "dat", "op", "voor", "met", "zijn", "niet", "aan", "er", "ook",
            "bij", "maar", "om", "uit", "door", "over", "dit", "deze", "wordt", "worden", "heeft",
            "hebben", "nog", "naar", "tot", "wel", "geen", "hun", "ik", "hij", "de", "en", "is",
            "als",
        ],
    ),
    (
        Interface::Swedish,
        &[
            "och", "att", "det", "som", "är", "på", "av", "för", "med", "till", "har", "inte",
            "ett", "han", "hon", "vi", "från", "men", "eller", "sig", "ska", "vid", "också",
            "efter", "där", "när", "denna", "detta", "alla", "mycket", "bara", "hade", "blir",
            "en", "den", "om",
        ],
    ),
    (
        Interface::Catalan,
        &[
            "els", "les", "uns", "unes", "dels", "és", "amb", "per", "més", "però", "també",
            "això", "aquest", "aquesta", "aquests", "són", "hi", "molt", "quan", "perquè", "fins",
            "seva", "seu", "ens", "el", "la", "de", "que", "no", "com",
        ],
    ),
    (
        Interface::Basque,
        &[
            "eta", "da", "bat", "ez", "dira", "dute", "zen", "ziren", "izan", "hau", "hori",
            "hura", "bezala", "baina", "edo", "ere", "gabe", "dago", "daude", "baino", "behar",
            "oso", "zein", "nola", "bere", "den", "dela", "zuen", "ditu", "dituzte",
        ],
    ),
    (
        Interface::Polish,
        &[
            "że", "nie", "się", "jest", "jak", "po", "co", "ale", "czy", "od", "przez", "dla",
            "tak", "już", "są", "był", "była", "było", "być", "ma", "mają", "może", "który",
            "która", "które", "tego", "ten", "tym", "gdy", "lub", "oraz", "tylko", "także",
            "bardzo", "jego", "jej", "ich",
        ],
    ),
];

/// Function words that are noise in a query and are not detection vocabulary.
///
/// [`STOP_WORDS`] exists to recognise a language, and adding to it changes
/// [`detect`] for every memory and every short title: `how` would count as
/// English evidence, and a two-word Spanish note already falls back to English.
/// So the words a *question* carries and a memory does not live here instead.
///
/// These are interrogatives, auxiliaries and possessives that a bag-of-words
/// search matches against every memory that happens to use them. They are not a
/// complete list of English or Spanish function words, for the reason the table
/// above gives: both are the commonest ones, not a vocabulary.
pub const QUERY_STOP_WORDS: &[&str] = &[
    // English. The table already holds `what`, `which`, `when`, `that`, `this`,
    // `there`, `is`, `are`, `was`, `were`, and the rest of its list; these are
    // what a question adds on top of it.
    "a", "an", "do", "does", "did", "how", "many", "much", "who", "whom", "whose", "where", "why",
    "my", "your", "our", "their", "its", "me", "him", "her", "us", "them", "i",
    // Spanish, with the accents a person types. The unaccented spellings are
    // either in the table (`que`, `como`, `cuando`) or are not words a query
    // uses; `cual`, `cuanto` and `quien` are here beside their accented forms
    // because a caller may type either.
    "qué", "cuál", "cual", "cuáles", "cuales", "cómo", "cuándo", "dónde", "donde", "quién", "quien",
    "quiénes", "cuánto", "cuanto", "cuánta", "cuanta", "cuántos", "cuantos", "cuántas", "cuantas",
    "mi", "mis", "tu", "tus", "nuestro", "nuestra", "vuestro", "vuestra",
];

/// Whether a query word carries no subject of its own.
///
/// The union of every language's [`STOP_WORDS`] and [`QUERY_STOP_WORDS`], not
/// one language's list chosen by detection. A question is short: `fuga de
/// memoria` has one recognised word and `detect` answers English, so a check
/// gated on the detected language would keep `de` for the one query that is
/// entirely Spanish. A store also holds more than one language — the setting is
/// where new memories are written, not what the old ones are in — and the cost
/// of the union is a word that is a function word somewhere being dropped from
/// a query where it was content, which is rare and cheap against answering a
/// question with a memory that matched `de`.
///
/// Case-folded, because the tokenizer folds case: `The` and `the` are one term.
pub fn is_stop_word(word: &str) -> bool {
    let folded = word.to_lowercase();
    QUERY_STOP_WORDS.contains(&folded.as_str())
        || STOP_WORDS
            .iter()
            .any(|(_, words)| words.contains(&folded.as_str()))
}

/// How many words a language must be recognised by before it is believed.
///
/// Three and not one: a title and a line of body in English around a single
/// Spanish word is English, and a two-word note has no language to read. Below
/// this the answer is English, which gets `porter` alone and is the safe
/// default, not a claim that the text is English.
const MINIMUM_HITS: usize = 3;

/// The leader must have this many hits to every two of the runner-up's, which is
/// half again as many. A text that mixes two languages evenly has no language,
/// and a wrong stemmer is worse than none: it stems the other half's words into
/// stems nobody queries.
const LEAD_NUMERATOR: usize = 3;
const LEAD_DENOMINATOR: usize = 2;

/// Each word that names exactly one language, with that language's position in
/// [`STOP_WORDS`].
fn recognised_words() -> &'static HashMap<&'static str, usize> {
    static INDEX: OnceLock<HashMap<&'static str, usize>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut owners: HashMap<&'static str, Vec<usize>> = HashMap::new();
        for (position, (_, words)) in STOP_WORDS.iter().enumerate() {
            for word in *words {
                owners.entry(word).or_default().push(position);
            }
        }
        owners
            .into_iter()
            .filter_map(|(word, positions)| match positions[..] {
                [only] => Some((word, only)),
                _ => None,
            })
            .collect()
    })
}

/// The language a memory is written in, read from its words.
///
/// Counts, for each candidate, the words of the title and the body that only that
/// language's list holds. The highest count wins when it is at least
/// [`MINIMUM_HITS`] and half again the runner-up's; anything else is English.
/// Identifiers, paths and code fall through uncounted, which is what lets a
/// Spanish sentence around `cargo test` and a function name still be Spanish.
pub fn detect(title: &str, content: &str) -> Interface {
    let recognised = recognised_words();
    let mut hits = [0_usize; STOP_WORDS.len()];
    for text in [title, content] {
        for word in text.split(|c: char| !c.is_alphanumeric()) {
            if word.is_empty() {
                continue;
            }
            if let Some(position) = recognised.get(word.to_lowercase().as_str()) {
                hits[*position] += 1;
            }
        }
    }
    let (leader, count) = hits
        .iter()
        .copied()
        .enumerate()
        .max_by_key(|&(_, count)| count)
        .unwrap_or((0, 0));
    let runner_up = hits
        .iter()
        .copied()
        .enumerate()
        .filter(|&(position, _)| position != leader)
        .map(|(_, count)| count)
        .max()
        .unwrap_or(0);
    if count >= MINIMUM_HITS && count * LEAD_DENOMINATOR >= runner_up * LEAD_NUMERATOR {
        STOP_WORDS[leader].0
    } else {
        Interface::English
    }
}

/// The code a row is recorded under.
pub fn code(language: Interface) -> &'static str {
    language.code()
}

/// The language a recorded code names, if it is one Leteo speaks.
pub fn from_code(code: &str) -> Option<Interface> {
    Interface::ALL
        .into_iter()
        .find(|language| language.code() == code)
}

/// The text as the language's stemmer reads it: lowercased words, stemmed,
/// separated by single spaces.
///
/// Words are cut where `unicode61` would cut them — at anything that is not a
/// letter or a digit — so a query term and an indexed word that the index would
/// have treated as one token are one token here too. A language with no
/// stemmer yields nothing, which is what leaves a row's stems empty.
pub fn stem_text(text: &str, language: Interface) -> String {
    let Some(algorithm) = algorithm(language) else {
        return String::new();
    };
    let stemmer = Stemmer::create(algorithm);
    let mut stemmed = String::with_capacity(text.len());
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        if !stemmed.is_empty() {
            stemmed.push(' ');
        }
        stemmed.push_str(&stemmer.stem(&word.to_lowercase()));
    }
    stemmed
}

/// Makes the three functions the stem triggers call available on a connection.
///
/// All three are needed by an insert or an edit of a memory's text, because the
/// triggers call them; a delete or an edit of `tool_name`, `type` or `project`
/// calls none. `named` is the language the setting names, or `None` when it
/// names none. `leteo_memory_language(title, content)` is what a row written on
/// this connection is recorded under: `named` when there is one, and the
/// language [`detect`] reads from the text when there is not. Both are fixed for
/// the life of the connection: a long-running server keeps stemming as it
/// opened until it is restarted. `leteo_stem_language()` answers the connection's
/// own language whatever the text says, English when none is named, and is what
/// the released migration 21 still calls.
///
/// Every connection that writes `observations` has to have registered these,
/// because the triggers call them. A foreign connection that does not fails the
/// write with "no such function" rather than leaving a row its index cannot find.
pub fn register(connection: &Connection, named: Option<Interface>) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    let connection_language = named.unwrap_or_default();
    connection.create_scalar_function("leteo_stem_language", 0, flags, move |_| {
        Ok(code(connection_language))
    })?;
    connection.create_scalar_function("leteo_memory_language", 2, flags, move |context| {
        if let Some(language) = named {
            return Ok(code(language));
        }
        let title: Option<String> = context.get(0)?;
        let content: Option<String> = context.get(1)?;
        Ok(code(detect(
            title.as_deref().unwrap_or_default(),
            content.as_deref().unwrap_or_default(),
        )))
    })?;
    connection.create_scalar_function("leteo_stem", 2, flags, |context| {
        let text: Option<String> = context.get(0)?;
        let code: Option<String> = context.get(1)?;
        Ok(match (text, code.as_deref().and_then(from_code)) {
            (Some(text), Some(language)) => stem_text(&text, language),
            _ => String::new(),
        })
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanish_inflections_share_a_stem() {
        let stems = |text| stem_text(text, Interface::Spanish);
        assert_eq!(stems("migración"), stems("migraciones"));
        assert_eq!(stems("ejecuta"), stems("ejecutó"));
        assert_eq!(stems("Migraciones, de datos"), "migracion de dat");
    }

    #[test]
    fn the_registered_function_stems_by_the_code_a_row_records() {
        let connection = Connection::open_in_memory().unwrap();
        register(&connection, Some(Interface::Spanish)).unwrap();
        let (language, stems, unknown): (String, String, String) = connection
            .query_row(
                "SELECT leteo_stem_language(), leteo_stem('ejecutó', 'es'), leteo_stem('ejecutó', 'zz')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(language, "es");
        assert_eq!(stems, "ejecut");
        assert_eq!(unknown, "");
    }

    #[test]
    fn a_text_is_detected_in_each_language_that_has_a_stemmer() {
        let cases = [
            (
                Interface::Spanish,
                "Los tests de integración también fallan cuando se ejecutan sin la base de datos, pero el problema es muy sencillo de resolver.",
            ),
            (
                Interface::Portuguese,
                "Os dados não foram guardados porque o servidor já estava muito ocupado e uma das tabelas tinha sido bloqueada pelo sistema.",
            ),
            (
                Interface::French,
                "Les tests ne passent pas dans cette version, mais ils sont très simples et le résultat est aussi plus stable.",
            ),
            (
                Interface::German,
                "Der Server wird nicht gestartet, wenn die Datei fehlt, und wir haben das Problem noch nicht gelöst.",
            ),
            (
                Interface::Italian,
                "Il server non risponde perché il database è molto lento e gli utenti hanno perso la sessione.",
            ),
            (
                Interface::Romanian,
                "Serverul nu pornește pentru că fișierul este foarte mare și baza de date a fost blocată după migrare.",
            ),
            (
                Interface::Dutch,
                "Het probleem is dat de server niet start omdat een bestand ontbreekt, maar we hebben het ook nog niet opgelost.",
            ),
            (
                Interface::Swedish,
                "Servern startar inte eftersom filen saknas och vi har inte hittat något som kan lösa det här problemet.",
            ),
            (
                Interface::Catalan,
                "Els usuaris també han perdut la sessió perquè la base de dades és molt lenta i hi ha poques còpies.",
            ),
            (
                Interface::Basque,
                "Zerbitzaria ez da hasten fitxategia falta delako, baina arazo hori oso erraza da eta ere konpontzen da.",
            ),
            (
                Interface::Polish,
                "Serwer nie działa, ponieważ plik jest uszkodzony, ale problem jest bardzo prosty i można go rozwiązać.",
            ),
        ];
        for (language, text) in cases {
            assert_eq!(detect("", text), language, "{text}");
        }
    }

    #[test]
    fn identifiers_around_a_spanish_sentence_do_not_make_it_english() {
        let text = "Para ejecutar las pruebas hay que usar `cargo test` y revisar el README, porque la función parse_config falla cuando el archivo es muy grande.";
        assert_eq!(detect("Cómo ejecutar los tests", text), Interface::Spanish);
    }

    #[test]
    fn technical_english_with_identifiers_is_english() {
        let text = "Run `cargo test` and check README.md, because parse_config returns an error when src/lib.rs is missing.";
        assert_eq!(detect("Config parsing", text), Interface::English);
    }

    #[test]
    fn a_text_too_short_to_tell_is_english() {
        assert_eq!(detect("Ejecuta el cron", "Cada noche."), Interface::English);
        assert_eq!(detect("", ""), Interface::English);
        assert_eq!(
            detect("Un fallo", "Es muy raro."),
            Interface::English,
            "two hits is below the minimum"
        );
    }

    #[test]
    fn a_lead_of_less_than_half_again_is_english() {
        let german = "der die nicht wenn";
        assert_eq!(
            detect("", &format!("los también sin pero {german}")),
            Interface::English,
            "four against four"
        );
        assert_eq!(
            detect("", &format!("los también sin pero muy {german}")),
            Interface::English,
            "five against four is less than half again"
        );
        assert_eq!(
            detect("", &format!("los también sin pero muy hay {german}")),
            Interface::Spanish,
            "six against four is exactly half again"
        );
    }

    #[test]
    fn a_word_that_two_languages_share_decides_for_neither() {
        let shared: Vec<&str> = recognised_words().keys().copied().collect();
        for (language, words) in STOP_WORDS {
            for word in words {
                let owners = STOP_WORDS
                    .iter()
                    .filter(|(_, other)| other.contains(word))
                    .count();
                assert_eq!(
                    shared.contains(word),
                    owners == 1,
                    "{word} in {}",
                    language.code()
                );
            }
        }
        assert_eq!(
            detect("", "de que de que de que de que de que"),
            Interface::English
        );
    }

    #[test]
    fn every_language_keeps_enough_words_of_its_own() {
        for (language, words) in STOP_WORDS {
            let own = words
                .iter()
                .filter(|word| recognised_words().contains_key(*word))
                .count();
            assert!(
                own >= 15,
                "{} keeps {own} words that name it alone",
                language.code()
            );
        }
    }

    #[test]
    fn the_table_covers_every_language_with_a_stemmer_and_english() {
        for language in Interface::ALL {
            let listed = STOP_WORDS.iter().any(|(listed, _)| *listed == language);
            let wanted = language == Interface::English || algorithm(language).is_some();
            assert_eq!(listed, wanted, "{}", language.code());
        }
    }

    /// A query word is dropped only when it carries no subject, in either
    /// language a question may be written in.
    ///
    /// The union is the point: `fuga de memoria` is too short for `detect` to
    /// call it Spanish, so a rule that asked the detected language would keep
    /// the one word the question does not mean.
    #[test]
    fn a_function_word_is_recognised_without_detecting_the_language() {
        for word in ["the", "de", "How", "MANY", "which", "cuál", "cómo", "dónde"] {
            assert!(is_stop_word(word), "{word} should be a stop word");
        }
        for word in [
            "memoria", "fuga", "límite", "subida", "requests", "minute", "email", "provider",
            "webhook",
        ] {
            assert!(!is_stop_word(word), "{word} carries a subject");
        }
    }

    /// The query list adds nothing to what recognises a language.
    ///
    /// Adding a word to `STOP_WORDS` changes what language every memory is read
    /// as, so the question words live in their own list. A word that is already
    /// detection vocabulary — `do` is Portuguese in the table — may be named
    /// here too, because `detect` never reads this list; what must hold is that
    /// a word added here alone cannot become language evidence.
    #[test]
    fn the_query_list_adds_no_evidence_to_language_detection() {
        for word in QUERY_STOP_WORDS {
            let already_detection = STOP_WORDS.iter().any(|(_, words)| words.contains(word));
            assert_eq!(
                recognised_words().contains_key(word),
                already_detection,
                "{word} must not be recognised by this list alone"
            );
        }
    }

    #[test]
    fn the_registered_language_function_answers_the_setting_or_the_text() {
        let spanish = "Los tests también fallan sin la base de datos, pero es muy sencillo.";
        let ask = |named: Option<Interface>, title: &str, content: &str| -> String {
            let connection = Connection::open_in_memory().unwrap();
            register(&connection, named).unwrap();
            connection
                .query_row(
                    "SELECT leteo_memory_language(?1, ?2)",
                    [title, content],
                    |row| row.get(0),
                )
                .unwrap()
        };
        assert_eq!(ask(None, "Nota", spanish), "es");
        assert_eq!(
            ask(None, "Note", "Runs the job and it will not stop."),
            "en"
        );
        assert_eq!(
            ask(Some(Interface::English), "Nota", spanish),
            "en",
            "a named language is not second-guessed"
        );
        assert_eq!(
            ask(Some(Interface::French), "Note", "It will not stop."),
            "fr"
        );
    }

    #[test]
    fn a_language_without_a_stemmer_yields_nothing() {
        assert_eq!(stem_text("migrations", Interface::English), "");
        assert_eq!(stem_text("migracións", Interface::Galician), "");
    }

    /// The three languages `rust-stemmers` does not carry are stemmed by the
    /// other source. Each pair is an inflection the memory index has to fold
    /// together, and the exact stem is pinned so a swap between the two
    /// implementations is a failing test rather than a silent re-index.
    #[test]
    fn catalan_basque_and_polish_inflections_share_a_stem() {
        assert_eq!(
            stem_text("configuració", Interface::Catalan),
            stem_text("configuracions", Interface::Catalan)
        );
        assert_eq!(stem_text("configuracions", Interface::Catalan), "configur");
        assert_eq!(
            stem_text("bilaketa", Interface::Basque),
            stem_text("bilaketak", Interface::Basque)
        );
        assert_eq!(stem_text("bilaketak", Interface::Basque), "bila");
        assert_eq!(
            stem_text("migracja", Interface::Polish),
            stem_text("migracje", Interface::Polish)
        );
        assert_eq!(stem_text("migracje", Interface::Polish), "migracj");
    }

    #[test]
    fn every_language_is_recorded_under_a_code_that_reads_back() {
        for language in Interface::ALL {
            assert_eq!(from_code(code(language)), Some(language));
        }
    }
}
