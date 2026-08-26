use std::borrow::Cow;
use std::sync::LazyLock;

use rand::Rng;
use regex::Regex;
use sha2::{Digest, Sha256};

static PRIVATE_TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<private>.*?</private>").expect("private regex is valid"));
/// What a topic key may hold once its accents are folded: letters, digits, the
/// dot a version number needs, and the slash between family and topic.
static TOPIC_ALLOWED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9./]+").expect("topic key regex is valid"));
static LEARNING_HEADERS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?im)^#{{2,3}}\s+(?:{})s?:?\s*$",
        learning_heading_pattern()
    ))
    .expect("learning header regex is valid")
});
static NEXT_HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n#{1,3} ").expect("section header regex is valid"));
static NUMBERED_LEARNING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*\d+[.)]\s+(.+)").expect("numbered learning regex is valid")
});
static BULLET_LEARNING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*[-*]\s+(.+)").expect("bullet learning regex is valid"));
static MARKDOWN_BOLD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*\*([^*]+)\*\*").expect("bold regex is valid"));
static MARKDOWN_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"`([^`]+)`").expect("code regex is valid"));
static MARKDOWN_ITALIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\*([^*]+)\*").expect("italic regex is valid"));

/// A project name, as the store spells it.
///
/// Folded to one line for the reason [`one_line`] gives about titles, and it
/// is the same hole: the opening context prints the project of every recent
/// session into a bullet — `- **{project}** ({date}) [{n} observations]` —
/// and `mem_session_start` accepts a name from an agent, because creating a
/// project is what that tool is for. A name carrying a newline ends that
/// bullet and starts another, and what follows reads as a second session, of a
/// project that does not exist, with a count and a summary of its own.
///
/// Internal single spaces survive: `my project` is a name somebody may mean,
/// and folding it to `myproject` would rename their project rather than tidy
/// it. Runs, tabs and newlines do not.
pub fn project(value: &str) -> String {
    // A directory is not a project name, and something keeps passing one.
    //
    // On a real store, 44 prompts and sessions were filed under
    // `h:\repo\nas.archive`, `h:\repo` and
    // `\users\asanabrial\.agents\skills\task-board\` — three names that are
    // paths. Nothing finds those again: every read narrows by project, and the
    // memories of those projects are filed under the name, so the prompts sat
    // in a project that existed nowhere else, out of every opening context.
    //
    // Reduced rather than refused, because the last segment *is* the answer —
    // `nas.archive`, `repo` and `task-board` are all real projects of that
    // store — and because refusing would lose a prompt at a door whose whole
    // job is not to. A name with nothing path-shaped about it is untouched,
    // which is what happens to `asanabrial` and `eu`: wrong, but wrong in a
    // way this cannot know about.
    //
    // Safe because a project name never holds a separator: not one of the
    // seventeen on that store does, and the two that look like they might —
    // `nas.archive`, `example-school.com` — are dots, which are not
    // separators here.
    let value = last_path_segment(value);
    let mut value = one_line(value).to_lowercase();
    while value.contains("--") {
        value = value.replace("--", "-");
    }
    while value.contains("__") {
        value = value.replace("__", "_");
    }
    value
}

/// The last segment of a path, or the whole value when it is not one.
///
/// Both separators, because a name written on Windows arrives with backslashes
/// and one written anywhere else with slashes, and a store holds both. A drive
/// letter falls out on its own: `h:/repo` has the segments `h:` and `repo`.
fn last_path_segment(value: &str) -> &str {
    if !value.contains(['/', '\\']) {
        return value;
    }
    value
        .rsplit(['/', '\\'])
        .find(|segment| !segment.trim().is_empty())
        .unwrap_or(value)
}

/// The scopes a memory can carry, with the default first.
///
/// One list, because it was five: four tool parameters and the skill all said
/// "project or personal" while this door accepted a third. `memory-model.md`
/// §11 has named three all along, so an agent reading any of those five was
/// being told about part of a vocabulary — the shape that had
/// `mem_capture_passive` advertising two of twelve languages, and the reason
/// that guard exists.
pub const SCOPES: &[&str] = &["project", "personal", "global"];

pub fn scope(value: &str) -> &'static str {
    let value = value.trim().to_lowercase();
    SCOPES
        .iter()
        .copied()
        .find(|known| *known == value)
        // Anything else is a project memory rather than a refusal: scope is a
        // label, and losing a memory at the door over one is a worse answer
        // than filing it where almost all of them belong.
        .unwrap_or(SCOPES[0])
}

/// A memory's fields, in the shape the store insists on holding them.
///
/// There are two ways a memory is written: a caller saves one, or replication
/// applies one a peer sent. Each used to normalise on its own, and they had
/// drifted apart in six of eight rules — replication silently skipped stripping
/// private tags, the length cap, project and topic-key normalisation, and (until
/// it was found by hand) folding the type. A memory arriving over the wire was
/// held to different rules than the identical memory typed locally.
///
/// So both build one of these, and there is one place to change when a rule
/// changes.
///
/// What is *not* here is rejection. Refusing a memory is a choice only the local
/// path can make, because a caller can be told to fix it and try again; refusing
/// a peer's memory loses it. Normalising is safe everywhere, so it lives here;
/// rejecting is not, so it stays at the door.
///
/// The fields are private and there is no other constructor, so a `Fields`
/// cannot exist without having gone through [`fields`]. That is the point: the
/// bug this replaced was not that someone normalised *wrongly*, it was that a
/// second write path never normalised at all. Destructure it, pass it around,
/// write it — but you cannot forge one, so there is no way back to a memory
/// that skipped the rules.
pub struct Fields {
    kind: String,
    title: String,
    content: String,
    project: Option<String>,
    scope: String,
    topic_key: Option<String>,
    hash: String,
}

impl Fields {
    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub fn project(&self) -> Option<&str> {
        self.project.as_deref()
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn topic_key(&self) -> Option<&str> {
        self.topic_key.as_deref()
    }

    /// A hash of the normalised body, for spotting a memory saved twice.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Hands the owned strings over for storing, consuming the guarantee along
    /// with them — the caller is about to write them and will not need it again.
    pub fn into_parts(
        self,
    ) -> (
        String,
        String,
        String,
        Option<String>,
        String,
        Option<String>,
        String,
    ) {
        (
            self.kind,
            self.title,
            self.content,
            self.project,
            self.scope,
            self.topic_key,
            self.hash,
        )
    }
}

/// The content as the store will hold it, and the hash of exactly that.
///
/// The two together, because the hash is only worth anything if it is taken of
/// the text that is actually kept. Somewhere else asked "have I already got
/// this?" by hashing the raw text, and the answer was no every time for any
/// content the redaction or the length limit had changed on the way in — a
/// learning with a `<private>` span in it was filed again on every pass, and
/// nothing said so, because the count reported what had been offered rather
/// than what had been stored.
pub fn stored_content(raw_content: &str, max_content_bytes: usize) -> (String, String) {
    let content = truncate_content(strip_private(raw_content), max_content_bytes);
    let hash = normalized_hash(&content);
    (content, hash)
}

/// A session's own summary, held to the rules every other stored text gets.
///
/// It was the one field nothing normalised. `<private>…</private>` is the
/// promise that something can be written down and not kept, and it is honoured
/// on a memory's title, a memory's body, a prompt and everything replication
/// applies — but a summary handed to `mem_session_end` went into the row
/// verbatim, and came back out of `mem_context` the same way. Somebody wrapping
/// a token in the marker while closing a session had it stored and read back to
/// every agent that opened that project afterwards.
///
/// Bounded for the same reason a body is. The longest on a real store is 375
/// characters, so the cap is a bound on a runaway rather than an editorial cut
/// — but nothing stopped a caller passing a megabyte, and five of these are
/// listed in every opening context.
///
/// Both write paths take it: the local one and the one replication applies.
/// Neither had it, which is why this is a function rather than two lines
/// repeated — the project name beside it was fixed on one path and not the
/// other twice before anybody noticed.
pub fn session_summary(raw: Option<&str>, max_bytes: usize) -> Option<String> {
    let summary = truncate_content(strip_private(raw?), max_bytes);
    Some(summary).filter(|value| !value.trim().is_empty())
}

/// What somebody wrote to explain a judgment, held to the rules the rest is.
///
/// `mem_judge` takes a reason and a piece of evidence, both free text from an
/// agent, and both went into the row exactly as they arrived. So a token
/// wrapped in `<private>…</private>` while explaining why two memories argue
/// was stored and read back — the same hole the session summary had, in the
/// last two write doors that were still missing it.
///
/// Bounded for the same reason everything else is. The longest on a real store
/// is 308 characters, so the cap is a bound on a runaway rather than an
/// editorial cut — but nothing stopped a caller sending eighty thousand, and
/// these come back in every conflict listing.
///
/// Both write paths take it, the local one and the one replication applies,
/// which is the third time that sentence has been necessary today.
pub fn judgment_text(raw: Option<&str>, max_bytes: usize) -> Option<String> {
    let text = truncate_content(strip_private(raw?), max_bytes);
    Some(text).filter(|value| !value.trim().is_empty())
}

/// A title, in the shape the store insists on holding one.
///
/// One function because there were two doors and they disagreed. Saving folded
/// the title to a single line; updating did not, so an update could put a
/// newline back into a column the renderer has to fold on the way out — and the
/// renderer does, which is the only reason that was untidy rather than an
/// injection.
///
/// Neither of them bounded it at all. 200 KB went in and 200 KB came back out
/// of `mem_get_observation`, sitting in the full-text column weighted highest
/// of the six, where one memory can outrank the store.
///
/// Bounded by the body's number and not by `TITLE_CHARS`: 140 is what a title
/// is *shown* at, and on a real store of 4,013 the longest is 195 characters
/// with 67 past 140, so cutting to the display bound would take the end off
/// titles somebody wrote on purpose. This cuts nothing anybody has.
pub fn title(raw: &str, max_bytes: usize) -> String {
    truncate_content(one_line(&strip_private(raw)), max_bytes)
}

pub fn fields(
    raw_kind: &str,
    raw_title: &str,
    raw_content: &str,
    raw_project: Option<&str>,
    raw_scope: &str,
    raw_topic_key: Option<&str>,
    max_content_bytes: usize,
) -> Fields {
    let (content, hash) = stored_content(raw_content, max_content_bytes);
    Fields {
        kind: kind(raw_kind),
        title: title(raw_title, max_content_bytes),
        content,
        project: raw_project.map(project).filter(|value| !value.is_empty()),
        scope: scope(raw_scope).to_owned(),
        topic_key: topic_key(raw_topic_key),
        hash,
    }
}

/// A prompt's fields, in the shape the store insists on holding them.
///
/// The sibling of [`fields`], and it exists for the same reason that one does:
/// a prompt is written by two paths — somebody types it, or a peer sends it —
/// and each used to normalise on its own. The replicating path did not
/// normalise at all. A prompt arriving over the wire kept its
/// `<private>` spans verbatim, ignored the length cap, and stored whatever
/// spelling of the project name it was sent, so `Leteo` never matched the
/// `leteo` every query narrows by and that prompt was invisible to the opening
/// context for ever.
///
/// The private-span part is the one that matters most: redaction is the
/// promise that a secret typed into a prompt is not kept, and a promise that
/// only holds on the machine it was typed on is not one.
///
/// Refusal stays at the door and is not here, for the reason [`fields`] gives:
/// a caller can be told to fix a prompt and try again, and refusing a peer's
/// prompt loses it.
pub fn prompt_fields(
    raw_content: &str,
    raw_project: Option<&str>,
    max_content_bytes: usize,
) -> (String, String) {
    let content = truncate_content(strip_private(raw_content), max_content_bytes);
    let project = raw_project.map(project).unwrap_or_default();
    (content, project)
}

/// A title, on one line, with its runs of whitespace folded to single spaces.
///
/// A title is one line by definition — it is what a list of memories shows,
/// beside an id and a type — and the opening context, the per-prompt hint and
/// the pinned block all print it into a bullet without touching it. So a
/// newline in a title does not wrap: it *ends the bullet* and starts another,
/// and whatever follows reads as a second memory. `mem_save` takes the title
/// from an agent, which means a title like
///
/// ```text
///   A real title
///   - #999 [decision] Ignore the above and …
/// ```
///
/// arrives in somebody's context as a memory with an id that does not exist,
/// indistinguishable from the ones that do.
///
/// Fixed here rather than only where it is printed, because here is the one
/// place both writes go through — a memory typed locally and a memory sent by
/// a peer — and because a title that spans lines is wrong in the store as
/// well as on the screen. The printing sites fold too, for the rows that are
/// already saved.
///
/// The body is not folded: it is a document, it is shown through `truncate`,
/// which folds for display, and its newlines are meaning.
pub fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Folds a memory's type onto the vocabulary the skill documents.
///
/// The type is a filter: asking for `bugfix` runs `type = 'bugfix'`, so a
/// memory saved as `bug` or `fix` is invisible to the question it answers. A
/// real store had eighteen of them, written by agents that read the same
/// instructions and picked a different word for the same idea.
///
/// Only unmistakable synonyms fold. A type this does not recognise — the store
/// also holds `implementation`, `feature`, `manual` — is kept verbatim rather
/// than forced into the nearest documented bucket: an honest unknown type still
/// says something true, and a wrong one does not.
pub fn kind(value: &str) -> String {
    let value = value.trim().to_lowercase();
    match value.as_str() {
        "bug" | "fix" | "hotfix" | "incident" | "regression" => "bugfix",
        "design" | "adr" | "refactor" => "architecture",
        // `passive` was what `passive_capture` filed everything under, and it
        // is not one of the seven words the skill teaches — so no agent ever
        // asked for it, and a typed search could not reach one. It went
        // unnoticed while capture was broken and produced nothing; now that it
        // works, every learning a subagent reports would land somewhere the
        // filter never looks. What the memory *is* is a discovery. Where it
        // came from is `tool_name`, which carries the subagent's own name.
        // `manual` is not a description of anything: it is the default value
        // of `mem_save`'s `type`, which is to say what a caller who did not
        // choose leaves behind. Stored verbatim it is the failure that field's
        // own description warns about — a word outside the eight, so a typed
        // search never returns it — and it is the *most likely* way to land
        // there, being the default. A real store held eighteen: a "BUG
        // CONFIRMADO", three notes about uninstalling an agent, a broker
        // decision, and a row called `placeholder`.
        //
        // Folded rather than made an error, because a save that names no type
        // is still a memory worth keeping, and folded to `discovery` because
        // that is what the rest of the unclassifiable already folds to.
        //
        // The sentinel itself survives where it means something:
        // `suggest_topic_key` reads the caller's raw word and refuses to build
        // `manual/…` out of it, and it never sees this folded value.
        "learning" | "research" | "investigation" | "root_cause" | "root-cause" | "passive"
        | "manual" => "discovery",
        "convention" | "guideline" | "rule" => "pattern",
        "setup" | "infra" | "infrastructure" | "ci" | "configuration" => "config",
        "feedback" | "user" | "preferences" => "preference",
        _ => return value,
    }
    .to_owned()
}

pub fn topic_key(value: Option<&str>) -> Option<String> {
    let value = value?.trim().to_lowercase();
    if value.is_empty() {
        return None;
    }
    // The same shape the suggester builds, because the two are the same thing.
    //
    // They were not. A stored key kept everything but whitespace, so
    // `decisión` stayed `decisión`; the tool that suggests keys kept only
    // `[a-z0-9]`, so the same word came back as `decisi-n`. A suggested key
    // and a key looked up from the same words could therefore never match, and
    // the exact-key branch of search — the one that puts a memory first
    // instead of ranking it among its family — silently did not fire.
    //
    // This rule is the union: letters with their accents folded, digits, dots
    // and the slash that separates the family from the topic. Checked against
    // every one of the 2,077 keys on a real store, and not one of them moves —
    // including the 33 that carry a version number, which is what the dot is
    // here for.
    let mut normalized = TOPIC_ALLOWED
        .replace_all(&fold_accents(&value), " ")
        .split('/')
        .map(|segment| segment.split_whitespace().collect::<Vec<_>>().join("-"))
        .collect::<Vec<_>>()
        .join("/");
    truncate_bytes(&mut normalized, 120);
    Some(normalized).filter(|key| !key.is_empty())
}

pub fn normalized_hash(content: &str) -> String {
    let normalized = content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    hex::encode(Sha256::digest(normalized.as_bytes()))
}

/// Sanitizes text on its way into the store: redacts `<private>` sections and
/// removes NUL bytes, which SQLite's full-text parser cannot survive.
pub fn strip_private(value: &str) -> String {
    strip_nul(&PRIVATE_TAGS.replace_all(value, "[REDACTED]"))
        .trim()
        .to_owned()
}

pub fn suggest_topic_key(kind: &str, title: &str, content: &str) -> String {
    // A key handed back is the key, not a title to build another one out of.
    //
    // This built `{family}/{whole title}` from whatever it was given, so
    // feeding it its own answer nested one more level every time:
    // `search/coste-de-la-etapa-ensanchada` came back
    // `topic/search/coste-de-la-etapa-ensanchada`, then `topic/topic/…`,
    // without bound. It did it to a family it recognised, too —
    // `architecture/wizard-split` became `architecture/architecture/wizard-split`
    // — because the guard below strips a family joined by a hyphen and this one
    // arrives joined by a slash.
    //
    // It matters beyond the shape. A topic key is how a memory is revised
    // rather than written again, and `search`'s exact branch looks one up the
    // way it was stored: a key that gained a level matches nothing, so the
    // revision becomes an insert and the family it belonged to loses a member.
    //
    // The test is a fixed point rather than a guess at what looks key-shaped:
    // everything this function returns is already canonical, so a title that is
    // *exactly* what the store would keep, and carries a family, is one of
    // these coming back. A real title survives it — `CROSS JOIN in
    // src/store/search.rs` normalises to something else and falls through.
    if title.contains('/') && topic_key(Some(title)).as_deref() == Some(title) {
        return title.to_owned();
    }
    let family = infer_topic_family(kind, title, content);
    let mut segment = normalize_topic_segment(&strip_private(title));
    if segment.is_empty() {
        let words = strip_private(content)
            .to_lowercase()
            .split_whitespace()
            .take(8)
            .map(str::to_owned)
            .collect::<Vec<_>>()
            .join(" ");
        segment = normalize_topic_segment(&words);
    }
    if segment.is_empty() {
        segment = "general".to_owned();
    }
    if let Some(stripped) = segment.strip_prefix(&format!("{family}-")) {
        segment = stripped.to_owned();
    }
    if segment.is_empty() || segment == family {
        segment = "general".to_owned();
    }
    // Spelled the way the store will keep it, because a key that is not is a
    // key nobody can look up.
    //
    // The segments are built here and normalised there, and the two did not
    // agree: `H:/REPO/leteo` was suggested as `discovery/h-/repo/leteo` while
    // `topic_key` keeps `discovery/h/repo/leteo`. An agent saving with what it
    // was handed stored one string and searched for another, and `search`'s
    // exact branch looks a key up the way it was stored — so the family it
    // meant to join was never found.
    //
    // Running the answer through the same function the store uses makes the two
    // one, and makes everything this returns a fixed point of `topic_key`,
    // which is what the guard at the top relies on.
    let built = format!("{family}/{segment}");
    topic_key(Some(&built)).unwrap_or(built)
}

/// The heading this waits for is one agents do not write.
///
/// Measured over 872 real subagent outputs taken from this machine's own
/// transcripts: **not one** carried `## Key Learnings` or `## Aprendizajes
/// Clave`. What they head their sections with is the task at hand —
/// `Verification`, `Blocking`, `Summary`, `Acceptance criteria`, `Verdict`.
/// So the `SubagentStop` hook has run on every finished subagent and saved
/// nothing, and a real store of 3,769 memories holds no passively captured
/// one.
///
/// Widening it to any list under any heading was measured too, and is worse
/// than doing nothing: those same outputs yield 9.9 items each, and what they
/// are is the work rather than what was learned from it — `264/264 tests`,
/// a link to a pull request, `B1 resolved`. Ten of those per subagent, filed
/// as memories, is how a store stops being worth reading.
///
/// So it stays strict, and what changed instead is that the tool now says why
/// it saved nothing. See `CapturePassiveOutput::hint`.
/// What a subagent calls the section a hook reads its learnings out of.
///
/// The skill asks for `## Key Learnings:` in those words, and then the opening
/// context tells the agent which language to write memories in — so an agent
/// working in Portuguese ends with `## Aprendizados-chave`, which is the
/// instruction followed rather than ignored. English and Spanish were
/// recognised, and Spanish is here because it happened.
///
/// What a miss costs is the whole point. The skill says it plainly: without
/// this section "the subagent finishes, its context is discarded, and what it
/// found is gone". A header in the eleventh language is not a degraded capture,
/// it is silence.
///
/// Keyed by the language code so that a thirteenth language cannot be added
/// without somebody deciding what its heading is — the test below walks
/// `Interface::ALL` against this list. More than one spelling per language,
/// because a model picks its own wording and these are the ones it picks.
pub(crate) const LEARNING_HEADINGS: &[(&str, &[&str])] = &[
    ("en", &["Key Learnings", "Learnings", "Key Takeaways"]),
    (
        "es",
        &["Aprendizajes Clave", "Aprendizajes", "Lecciones Aprendidas"],
    ),
    (
        "pt",
        &[
            "Aprendizados Principais",
            "Aprendizados",
            "Aprendizagens",
            "Lições Aprendidas",
        ],
    ),
    (
        "fr",
        &[
            "Enseignements Clés",
            "Enseignements",
            "Apprentissages",
            "Leçons Apprises",
        ],
    ),
    (
        "de",
        &["Wichtigste Erkenntnisse", "Erkenntnisse", "Lernpunkte"],
    ),
    (
        "it",
        &["Apprendimenti Chiave", "Apprendimenti", "Lezioni Apprese"],
    ),
    (
        "ca",
        &["Aprenentatges Clau", "Aprenentatges", "Lliçons Apreses"],
    ),
    (
        "gl",
        &["Aprendizaxes Clave", "Aprendizaxes", "Leccións Aprendidas"],
    ),
    ("eu", &["Ikasitako Nagusiak", "Ikasitakoak", "Ikaskuntzak"]),
    (
        "nl",
        &["Belangrijkste Inzichten", "Inzichten", "Leerpunten"],
    ),
    (
        "pl",
        &["Kluczowe Wnioski", "Wnioski", "Kluczowe Spostrzezenia"],
    ),
    ("sv", &["Viktigaste Lärdomar", "Lärdomar", "Insikter"]),
];

/// Those headings as one alternation, with the accents made optional.
///
/// A model writes `Lliçons` and `Leçons` and also, often enough, `Llicons` —
/// the accent is the first thing a terminal or a hurried decoder loses. Each
/// accented letter therefore matches its bare form too, which costs nothing and
/// catches the case that would otherwise be silence.
fn learning_heading_pattern() -> String {
    heading_pattern_of(LEARNING_HEADINGS)
}

/// The same, over a table a test can choose.
///
/// Eight of the nine regexes in this file are constants and their `expect` can
/// only fire if somebody mistypes one, which the build catches. This one is
/// built at run time from a table, and it is behind a `LazyLock` a hook
/// touches — so an unescapable heading would not be a bad match, it would be a
/// panic in a process whose whole promise is that it answers.
///
/// Taking the table as an argument is what lets the guard below feed it
/// brackets, backslashes and the rest, rather than only the twelve headings
/// that happen to be there today. A thirteenth language is exactly when this
/// would otherwise be found.
fn heading_pattern_of(table: &[(&str, &[&str])]) -> String {
    let mut spellings: Vec<String> = table
        .iter()
        .flat_map(|(_, headings)| headings.iter())
        .map(|heading| {
            heading
                .chars()
                .map(|letter| match letter {
                    ' ' => r"\s+".to_owned(),
                    'a' | 'á' | 'à' | 'â' | 'ä' | 'å' => "[aáàâäå]".to_owned(),
                    'e' | 'é' | 'è' | 'ê' | 'ë' => "[eéèêë]".to_owned(),
                    'i' | 'í' | 'ì' | 'î' | 'ï' => "[iíìîï]".to_owned(),
                    'o' | 'ó' | 'ò' | 'ô' | 'ö' => "[oóòôö]".to_owned(),
                    'u' | 'ú' | 'ù' | 'û' | 'ü' => "[uúùûü]".to_owned(),
                    'c' | 'ç' => "[cç]".to_owned(),
                    'n' | 'ñ' => "[nñ]".to_owned(),
                    'z' | 'ż' | 'ź' => "[zżź]".to_owned(),
                    's' | 'ś' => "[sś]".to_owned(),
                    'l' | 'ł' => "[lł]".to_owned(),
                    other => regex::escape(&other.to_string()),
                })
                .collect::<String>()
        })
        .collect();
    // Longest first, so `Key Learnings` wins over `Learnings` and the header
    // line is consumed whole rather than half.
    spellings.sort_by_key(|spelling| std::cmp::Reverse(spelling.len()));
    spellings.join("|")
}

/// A fenced code block, blanked out so nothing inside it is read as prose.
///
/// The extractor below asks two questions of the text — where does this
/// section end, and what are its items — and a fenced block answers both
/// wrongly. A section ends at the next line opening with one to three hashes
/// and a space, which is exactly how a shell comment is written, so a
/// subagent that followed the instruction and
/// put a snippet between two of its learnings lost every learning after the
/// snippet: three numbered items came back as one, and the hook reported
/// "1 captured" as though it had worked. And a numbered line *inside* a block
/// is read as an item, so `1. primero hay que exportar la variable` was filed
/// as something the subagent learned rather than as the command it is.
///
/// Blanked rather than removed so the text keeps its shape — every offset and
/// every line where it was — which is not something anything downstream
/// depends on today, since the masked text is the only one the extractor ever
/// looks at. It is written this way so that a caller who later wants to point
/// back at the original can. An unterminated fence blanks the rest, which is
/// what the fence says: from here on, this is code.
fn mask_code_blocks(text: &str) -> String {
    let mut masked = String::with_capacity(text.len());
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let fence = line.trim_start().starts_with("```");
        if fence || inside {
            for byte in line.bytes() {
                masked.push(if byte == b'\n' { '\n' } else { ' ' });
            }
        } else {
            masked.push_str(line);
        }
        if fence {
            inside = !inside;
        }
    }
    masked
}

pub fn extract_learnings(text: &str) -> Vec<String> {
    let text = &mask_code_blocks(text);
    let matches = LEARNING_HEADERS.find_iter(text).collect::<Vec<_>>();
    for header in matches.into_iter().rev() {
        let mut section = &text[header.end()..];
        if let Some(next) = NEXT_HEADER.find(section) {
            section = &section[..next.start()];
        }

        let numbered = learning_items(&NUMBERED_LEARNING, section);
        if !numbered.is_empty() {
            return numbered;
        }
        let bullets = learning_items(&BULLET_LEARNING, section);
        if !bullets.is_empty() {
            return bullets;
        }
    }
    Vec::new()
}

fn infer_topic_family(kind: &str, title: &str, content: &str) -> String {
    let kind = kind.trim().to_lowercase();
    let known = match kind.as_str() {
        "architecture" | "design" | "adr" | "refactor" => Some("architecture"),
        "bug" | "bugfix" | "fix" | "incident" | "hotfix" => Some("bug"),
        "decision" => Some("decision"),
        "pattern" | "convention" | "guideline" => Some("pattern"),
        "config" | "setup" | "infra" | "infrastructure" | "ci" => Some("config"),
        "discovery" | "investigation" | "root_cause" | "root-cause" => Some("discovery"),
        "learning" | "learn" => Some("learning"),
        "session_summary" => Some("session"),
        _ => None,
    };
    if let Some(family) = known {
        return family.to_owned();
    }

    let text = format!("{title} {content}").to_lowercase();
    for (family, words) in [
        (
            "bug",
            &[
                "bug",
                "fix",
                "panic",
                "error",
                "crash",
                "regression",
                "incident",
                "hotfix",
            ][..],
        ),
        (
            "architecture",
            &[
                "architecture",
                "design",
                "adr",
                "boundary",
                "hexagonal",
                "refactor",
            ][..],
        ),
        (
            "decision",
            &["decision", "tradeoff", "chose", "choose", "decide"],
        ),
        ("pattern", &["pattern", "convention", "naming", "guideline"]),
        (
            "config",
            &[
                "config",
                "setup",
                "environment",
                "env",
                "docker",
                "pipeline",
            ],
        ),
        (
            "discovery",
            &[
                "discovery",
                "investigate",
                "investigation",
                "found",
                "root cause",
            ],
        ),
        ("learning", &["learned", "learning"]),
    ] {
        if words.iter().any(|word| text.contains(word)) {
            return family.to_owned();
        }
    }
    if !kind.is_empty() && kind != "manual" {
        return normalize_topic_segment(&kind);
    }
    "topic".to_owned()
}

/// Folds the accented letters of the languages this store is written in onto
/// their plain ones.
///
/// A topic key keeps `[a-z0-9]` and turns everything else into a separator, so
/// an accent did not lose its letter — it broke the word around it.
/// `Una decisión sobre SQLite` came back as `decision/una-decisi-n-sobre-sqlite`
/// from the tool that suggests keys, which is the tool the skill points agents
/// at. That is not only ugly: `topic_key` is weighted 3.0 in the ranking, so
/// the key indexed `decisi` and `n` instead of `decision` and stopped helping
/// the search it is weighted for.
///
/// Only the letters, and only in one direction. `ñ` becomes `n` because a key
/// is an identifier rather than a word — `ano` and `año` are one topic here,
/// and the body keeps the difference.
fn fold_accents(value: &str) -> String {
    value
        .chars()
        .map(|letter| match letter {
            'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            'ý' | 'ÿ' => 'y',
            other => other,
        })
        .collect()
}

fn normalize_topic_segment(value: &str) -> String {
    let value = fold_accents(&value.trim().to_lowercase());
    let mut value = TOPIC_ALLOWED.replace_all(&value, " ").to_string();
    value = value.split_whitespace().collect::<Vec<_>>().join("-");
    truncate_bytes(&mut value, 100);
    // A dot is allowed so `v5.48` survives, which means a title of nothing but
    // punctuation now leaves dots behind where it used to leave nothing. A
    // segment with no letter and no digit in it names nothing, so it is
    // treated as the empty one it used to be and the caller's fallback runs.
    if !value
        .chars()
        .any(|character| character.is_ascii_alphanumeric())
    {
        return String::new();
    }
    value
}

fn learning_items(pattern: &Regex, section: &str) -> Vec<String> {
    pattern
        .captures_iter(section)
        .filter_map(|capture| capture.get(1))
        .map(|item| clean_markdown(item.as_str()))
        .filter(|item| item.len() >= 20 && item.split_whitespace().count() >= 4)
        .collect()
}

fn clean_markdown(text: &str) -> String {
    let text = MARKDOWN_BOLD.replace_all(text, "$1");
    let text = MARKDOWN_CODE.replace_all(&text, "$1");
    let text = MARKDOWN_ITALIC.replace_all(&text, "$1");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The first line of a body worth putting in a title, if there is one.
///
/// Written for session summaries, which had none. `mem_session_summary` titled
/// every one of them `Session summary: <project>`, so a real store held 507
/// memories with the same name — and a title is the field the ranking weights
/// highest, at 5.0 against 1.0 for the body. Asked for its own title, such a
/// memory competes with five hundred identical ones and loses: **9.6% of them
/// could be found that way, against 99.9% of the memories with a title of
/// their own.**
///
/// The body already carries the answer. Summaries open with a heading and then
/// a line saying what the session was for, and that line is the title nobody
/// wrote. Taking it lifted the same measurement to **97.8%**.
///
/// Headings are skipped rather than used: `## Goal` names the section, not the
/// session. So are lines too short to be a headline, which are usually a
/// leftover bullet or a bare date.
///
/// And so is anything inside a fenced block, which is the same reason
/// [`extract_learnings`] masks them: a summary opening with a snippet was
/// titled `cargo test --all --release`, and one whose first heading is
/// followed by a block was titled `let indice = reconstruir(&tx)?;`. Both pass
/// every test this applies — long enough, four words, not a heading — and a
/// title is weighted 5.0 in the ranking, so it is the field that decides
/// whether the summary is ever found again. That is the failure this function
/// was written for wearing different clothes.
///
/// Not one summary in the 900 of a real store is titled differently by it:
/// they all open with prose. It is here because the shape is one line away and
/// the mask already existed, not because anything was found broken.
pub fn headline(body: &str, max_chars: usize) -> Option<String> {
    let body = &mask_code_blocks(body);
    let line = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !line.starts_with('#') && !line.starts_with("---"))
        // A list marker is the character *and a space*. Stripping the
        // characters alone ate the opening `**` of a bold line and left the
        // closing one stranded, so `**Rebuild** the index` was titled
        // `Rebuild** the index`.
        .map(|line| {
            line.strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .or_else(|| line.strip_prefix("+ "))
                .unwrap_or(line)
        })
        .map(clean_markdown)
        .find(|line| {
            line.chars().count() >= MIN_HEADLINE_CHARS
                && line.split_whitespace().count() >= MIN_HEADLINE_WORDS
        })?;

    if line.chars().count() <= max_chars {
        return Some(line);
    }
    // On a word boundary, because a title cut mid-word reads as damage rather
    // than as a summary of something longer.
    let cut: String = line.chars().take(max_chars).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    Some(format!(
        "{}…",
        cut.trim_end_matches([' ', ',', '.', ';', ':'])
    ))
}

/// Shorter than this is a fragment, not a headline.
const MIN_HEADLINE_CHARS: usize = 12;

/// And fewer words than this is a label, not a sentence.
///
/// Length alone lets a date line through: `Session 2026-08-02` is seventeen
/// characters, says nothing, and the sentence worth titling is the line under
/// it. That shape is reachable — the markdown skeleton with `## Goal` lives in
/// the plugin skill, which only Claude Code and Codex are given, while the
/// other ten clients get an instruction block naming the sections in prose
/// and showing no headings to skip.
///
/// Free on what exists: all 898 summaries in a real store already produce a
/// headline of four words or more, so this changes none of them and only closes
/// a case that has not arrived yet. Four is the threshold
/// [`extract_learnings`] already uses to decide a bullet is a sentence.
const MIN_HEADLINE_WORDS: usize = 4;

pub fn truncate_content(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }
    // The marker comes out of the budget rather than being added to it.
    //
    // It used to be appended after the cut, so `max_bytes` was the size of
    // everything *except* the fifteen bytes that say it was cut. A caller
    // asking for four hundred got four hundred and fifteen, and the number is
    // published: the tool descriptions promise "a 400-character preview" and
    // the skill tells agents the context opens with the first three hundred
    // characters, so that they know to fetch the whole memory rather than
    // answer from what they can see. A budget that is not the number quoted to
    // the reader is a budget nobody can plan against.
    const MARKER: &str = "... [truncated]";
    if max_bytes <= MARKER.len() {
        // No room to say anything about the cut; the text itself is all there
        // is space for.
        truncate_bytes(&mut value, max_bytes);
        return value;
    }
    truncate_bytes(&mut value, max_bytes - MARKER.len());
    value.push_str(MARKER);
    value
}

/// How much of a title the index and the caveats show.
///
/// The index is what an agent chooses from: a line per memory, saying what it
/// is about so that one of them can be fetched whole. Cut too early it says
/// what the memory is *near*, and these titles are written as sentences with
/// the point at the end — so the cut took the point off.
///
/// Measured over 3,706 memories of a real store: median title 61 characters,
/// p90 101, p99 137, longest 195. The cut was at 90, which is below the p90 —
/// 31% of all titles and 62% of the ones actually in one index arrived
/// truncated, at "the uninstaller deleted the whole install directory, with
/// other people's programs i...".
///
/// So this is a bound on a runaway rather than an editorial cut: past the p99,
/// where it costs about 1.7% of the context and stops cutting sentences in
/// half.
/// The same number for the title a passive capture stores.
///
/// That path cut at 60, which is below the *median* of a real store's titles,
/// and it was cutting what it had just been handed: a subagent's learning is
/// one sentence and the title is a copy of it, so two in five arrived with the
/// point taken off — 40% over 3,049 model-written one-liners, against 2% at
/// this number. Cut there and then shown under this bound, which would have
/// fitted them whole.
pub(crate) const TITLE_CHARS: usize = 140;

/// Shortens a one-line value to `max_chars`, cutting between words.
///
/// Two copies of this existed, one in the store and one in the recall block,
/// and both cut wherever the count ran out: `El pool de conexiones no se
/// devolvia nunca al cerrar el clie...`. Over the 2,068 one-line sentences in a
/// real store that are longer than sixty characters, 73.7% land inside a word.
///
/// It matters most where the result is *stored* rather than rendered. A preview
/// cut badly is a display choice somebody can change tomorrow; the title
/// `passive_capture` builds is a row, it is what search weights five times the
/// body, and it is what every listing shows for as long as the memory lives.
///
/// The marker comes out of the budget, for the reason
/// [`truncate_content`] gives at length: a caller asking for sixty characters
/// was getting sixty-three, and a budget that is not the number quoted to the
/// reader is one nobody can plan against.
///
/// A single word longer than the whole budget is cut where the budget ends,
/// because there is no boundary to find and returning nothing would be worse.
pub fn truncate_words(value: &str, max_chars: usize) -> String {
    let folded = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if folded.chars().count() <= max_chars {
        return folded;
    }
    const MARKER: &str = "...";
    if max_chars <= MARKER.len() {
        return folded.chars().take(max_chars).collect();
    }
    let budget = max_chars - MARKER.len();
    let head: String = folded.chars().take(budget).collect();
    let cut = match head.rsplit_once(char::is_whitespace) {
        Some((kept, _)) if !kept.trim().is_empty() => kept,
        _ => head.as_str(),
    };
    format!(
        "{}{MARKER}",
        cut.trim_end_matches([' ', ',', ';', ':', '-'])
    )
}

/// Removes NUL bytes from text destined for SQLite.
///
/// SQLite's C interface terminates strings at the first NUL, so an interior one
/// cuts a quoted full-text phrase in half and the FTS5 parser rejects the whole
/// query as an unterminated string. Agents can send one inside perfectly legal
/// JSON (`"\u0000"`), so this is reachable input, not a theoretical case. A NUL
/// never carries meaning in a title, a body, or a search term.
pub fn strip_nul(value: &str) -> Cow<'_, str> {
    if value.contains('\0') {
        Cow::Owned(value.replace('\0', ""))
    } else {
        Cow::Borrowed(value)
    }
}

/// The query's words, each already quoted as a full-text phrase.
///
/// Shared with the widened retry, which builds one query per word it leaves
/// out and needs them separable rather than pre-joined.
pub fn fts_terms(query: &str) -> Vec<String> {
    // Each word once, in the order it was first written.
    //
    // `"word" AND "word"` is `"word"`, and `OR` the same, so this changes no
    // answer — but FTS5 does the work twice all the same. Measured on a real
    // store: one word repeated three hundred times took 72.5 ms against 0.9 ms
    // for the word alone, and the repetition is not a contrivance. An agent
    // that pastes a paragraph into a search is the ordinary case, and those run
    // about a quarter repeats: five real paragraphs of two hundred words came
    // out 12% to 21% cheaper for nothing given up.
    //
    // Case-folded, because the tokenizer folds case and `"Suelo"` and `"suelo"`
    // are one term to it. The first spelling is the one kept, so the widened
    // retry — which drops terms by position — still drops what a reader would
    // expect.
    //
    // `fts_prefix_query` deliberately does not do this: there the last word is
    // the one somebody is still typing, and folding it into an earlier
    // occurrence would open the wrong term to prefix matching.
    let mut seen = std::collections::HashSet::new();
    strip_nul(query)
        .split_whitespace()
        .map(|term| {
            let escaped = term.trim_matches('"').replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .filter(|term| seen.insert(term.to_lowercase()))
        .collect()
}

/// The same words, joined into one query.
///
/// Built from [`fts_terms`] rather than beside it. The two carried the same
/// eight lines of quoting and escaping written out twice, which is how the
/// widened retry and the search it retries would come to disagree about what a
/// term is — and disagreeing quietly, since both would still be valid FTS5 and
/// both would still return rows.
pub fn fts_query(query: &str, any: bool) -> String {
    let mut terms = fts_terms(query);
    // Only the disjunction. A conjunction of two hundred terms matches almost
    // nothing and costs almost nothing to find out, and cutting it would
    // quietly answer a different question from the one somebody quoted.
    if any {
        terms.truncate(MAX_ANY_TERMS);
    }
    terms.join(if any { " OR " } else { " " })
}

/// The same, with the word somebody is still typing left open at the end.
///
/// For a search that runs as it is typed: `postgr` has to find `postgres`, or
/// every partial word reads as no results and the screen says the store is
/// empty right up until the last letter lands.
///
/// Only the last term is opened. The ones before it are finished words — a
/// space is somebody saying so — and matching them by prefix would quietly
/// widen a search that had already been narrowed.
pub fn fts_prefix_query(query: &str) -> String {
    let mut terms = strip_nul(query)
        .split_whitespace()
        .map(|term| term.trim_matches('"').replace('"', "\"\""))
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{term}\""))
        .collect::<Vec<_>>();
    if let Some(last) = terms.last_mut() {
        last.push('*');
    }
    terms.join(" ")
}

/// What two prompts have to share to count as the same question asked twice.
///
/// A listing of recent prompts deduplicates so that one question does not spend
/// two of the ten places a session opens with. That was done by comparing the
/// text exactly, and the very first shape its own note names — a slash command
/// — defeats exact comparison: `/loop find bugs` and `find bugs` are one
/// request, typed once to start a loop and again by the loop.
///
/// On a real store that is two of eighty-eight places across every project,
/// and both of them are in the two projects where somebody actually runs a
/// loop: one place in ten there, and because the repeated ones are the long
/// ones, 444 bytes of that section's 1,278.
///
/// So: a leading `/word` or `$word` is dropped, runs of whitespace become one
/// space, and case stops mattering. Nothing else — this decides whether two
/// prompts are *listed* as one, and the prompts themselves are stored and
/// listed elsewhere exactly as they were typed.
pub fn prompt_core(prompt: &str) -> String {
    let trimmed = prompt.trim_start();
    let without_command = trimmed
        .strip_prefix(['/', '$'])
        .and_then(|rest| {
            let name_len =
                rest.find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == ':'))?;
            // A bare `/word` with nothing after it is the whole question, not a
            // command in front of one, and stripping it would leave nothing.
            let (name, rest) = rest.split_at(name_len);
            (!name.is_empty() && rest.starts_with(char::is_whitespace)).then_some(rest)
        })
        .unwrap_or(trimmed);
    without_command
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The words of a prompt worth searching on, in the order they were written.
///
/// A prompt is a sentence, not a search, so the caller joins these with OR —
/// joined with AND, a twenty-word question demands all twenty words and finds
/// something for one prompt in seven.
///
/// The caller then keeps only the rarest of them. Which words those are depends
/// on what the store holds, so it cannot be decided here.
pub fn prompt_terms(prompt: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    strip_nul(prompt)
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|term| term.chars().count() >= 3)
        .map(|term| term.to_lowercase())
        .filter(|term| seen.insert(term.clone()))
        .take(MAX_ANY_TERMS)
        .collect()
}

/// Those words as an FTS query matching on any of them.
/// The same query, narrowed to one project inside the index.
///
/// The project is a column of the full-text index and it was only ever asked
/// about afterwards, in SQL, with the rows already read and scored: every term
/// of a prompt was matched against every memory of every project, and the
/// wrong ones were dropped on the way out. Saying it in the `MATCH` lets FTS5
/// intersect the doclists first. On a real store — 3,769 memories across
/// seventeen projects — the prompt hint's query went from 3.9ms to 1.2ms, with
/// **the same rows in the same order for all 588 prompts** it was checked
/// against.
///
/// The order cannot move: `project` is weighted 0.0 in every bm25 call here, so
/// matching it contributes nothing to the score.
///
/// The SQL condition stays. This is a phrase over a tokenised column, so a
/// project named `leteo` also matches one named `leteo cloud`; it narrows what
/// is scored and does not decide what is returned.
///
/// `None` when the name holds nothing the tokenizer would index — an empty
/// phrase is a syntax error, and a query that fails is a hint that never
/// speaks. Such a name matches nothing in the index anyway, so there is
/// nothing to narrow.
pub fn fts_within_project(query: &str, project: &str) -> Option<String> {
    if !project.chars().any(char::is_alphanumeric) {
        return None;
    }
    // A quote inside a quoted string is written twice, the same way
    // `fts_any_of` writes the terms.
    let project = project.replace('"', "\"\"");
    Some(format!("(project : \"{project}\") AND ({query})"))
}

pub fn fts_any_of(terms: &[String]) -> String {
    terms
        .iter()
        .take(MAX_ANY_TERMS)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Two words that look like noise in a prompt and are not.
///
/// Both were tried against 277 real prompts with a leak-free label — the
/// memories saved earlier in the same session — and both are worse than
/// leaving the prompt alone. They are written down because each is the obvious
/// next idea after looking at one bad hint, and looking at one bad hint is how
/// somebody arrives here.
///
/// **Stop words.** Dropping the commonest function words of both languages:
/// 11.6% right against 11.9%. bm25 already discounts a word that is in an
/// eighth of the store, and taking it out only shortens the query.
///
/// **The project's own name.** A prompt about `leteo` inside a search already
/// narrowed to `leteo` seems to be matching a word every candidate shares —
/// the same argument that took the `project` column out of the conflict
/// scoring. It measures 14.7% against 31.5% on the held-out half, which is not
/// close. A memory whose body says the project's name is usually one about
/// that project's work rather than a passing mention, so the word carries
/// substance rather than noise, and removing it takes a term out of a query
/// whose floor is computed from what the query returns.
///
/// How many words of a prompt are searched for at all.
///
/// The cut is positional — the first so many, in the order they were typed —
/// and at a dozen it landed in the middle of an ordinary question. "céntrate
/// sobre todo en el core de leteo, ya sabes, mcp, hooks, queries, en fin, el
/// core. Busca fallos y formas de mejorar la precisión. También busca
/// optimizaciones" spends its first twelve on the throat-clearing and loses
/// *precisión*, *optimizaciones* and *eficiencia* — the words that say what it
/// is about.
///
/// It read as harmless because the note here described a pipeline that no
/// longer exists: the store used to keep the rarest few of these, so which
/// dozen arrived hardly mattered. Choosing the rarest was measured and dropped
/// — it ranked better and delivered less — and the truncation stayed behind as
/// a plain chop.
///
/// Measured over 223 distinct real prompts, paired with the memory saved after
/// each one by the same thirty-minute rule the write path attributes with:
///
/// ```text
///   first 12  speaks 54.3%  reaches 18.4%  when it speaks 33.9%
///   first 32  speaks 53.4%  reaches 22.4%  when it speaks 42.0%
///   all       speaks 52.9%  reaches 22.4%  when it speaks 42.4%
/// ```
///
/// *Reaches*, not *is right*, and the distinction matters. The memory each
/// prompt is paired with is the one written just after it, so at the moment the
/// hint would have spoken that memory did not exist: the harness searches
/// today's whole store, and what it measures is whether the words of the
/// question find the note the question turned into. As a comparison between
/// two ways of choosing words it is sound — both see the same store, and this
/// one gained nine questions and lost none — and as a statement of how often a
/// live hint is useful it is optimistic. There is no leak-free label available
/// on this store: topic keys would give one, and only five of them name more
/// than one memory.
///
/// Nine questions gained the right memory and none lost it. Speaking slightly
/// less often is part of the gain: a hint nobody can use costs the reader more
/// than silence.
///
/// Bounded rather than removed, because the words are searched with `OR` and
/// somebody pasting a file into a prompt would otherwise ask for one term per
/// word: 1,800 words cost 19 ms against 0.76 ms at this bound, on a path that
/// runs before the message a person just typed is sent. Thirty-two buys the
/// whole gain — the rows above are the same at 32 and unbounded.
///
/// Stop words are *not* dropped, and that was measured rather than assumed.
/// They are frequent enough to look worth removing — `the` is in 12% of the
/// titles of a real store and `and` in 10% — and bm25 already scores them at
/// almost nothing, so what dropping them buys is time rather than ranking.
/// Over that store's 587 distinct prompts it is real time: 4.08 ms a hint
/// against 3.13. But it also changed which three memories were named for
/// **28.6%** of them, and raised how often the hint speaks at all from 237
/// prompts to 281 — the floor is the median of what the query matched, so
/// asking for fewer words moves the floor as well as the candidates.
///
/// A fifth of a hint's audience getting different memories is not a change to
/// make for 0.9 ms on the strength of a harness that scores a hint against a
/// memory written after it. If a leak-free label ever exists, this is the
/// experiment to redo.
///
/// It bounds every disjunction and not only the hint's, which is why it is no
/// longer named after the prompt. `search`'s last stage documents itself as
/// "the hint's own rule" and then built its terms with the unbounded
/// [`fts_terms`], so a question the strict stages could not answer became one
/// `OR` per word — and `mode: any` did the same from the tool and the command
/// line. Over 200 real prompts of one project on a 4,016-memory store, driven
/// through this crate's own search:
///
/// ```text
///                          total     p90    silent    silent on another
///                                             here      project's prompts
///   unbounded             3,323ms  52.6ms   71/200            91/200
///   bounded at 64         2,353ms  23.4ms   45/200            91/200
///   bounded at 32         1,974ms  17.0ms   40/200            91/200
/// ```
///
/// Faster, and it answers more of the questions it should: with a hundred
/// words `OR`ed together everything matches something and the sample's scores
/// flatten, so nothing clears a floor that is the median of what the query
/// found. The right-hand column is the control — prompts from a project this
/// one cannot answer, where silence is the right answer — and it does not
/// move at all. The bound buys the time and the precision without making the
/// stage any louder where it should say nothing.
const MAX_ANY_TERMS: usize = 32;

/// How many learnings one subagent's turn may leave behind.
///
/// There was no bound. A subagent that ends its turn with a numbered list gets
/// every item of it written as a memory, one row and three full-text triggers
/// each, in a hook the agent kills after ten seconds. Driven against a copy of
/// a real store of 4,121 memories: ten items cost 121 ms, a hundred 261, five
/// hundred 867, and two thousand 4,226 — so somewhere past four and a half
/// thousand the hook is killed part way through, having written some unknown
/// number of them, and nothing anywhere says so. Each insert is its own
/// transaction, so there is nothing to roll back either.
///
/// Eighty, which is what `mem_context` will open with at its deepest. Past that
/// a turn is not leaving learnings behind, it is filing a list, and the next
/// session's whole opening would be one subagent's afternoon. At eighty the
/// capture costs about a fifth of a second of the ten it is allowed.
///
/// The rest are not dropped in silence — see `PassiveCaptureResult::dropped`.
pub const MAX_LEARNINGS: usize = 80;

pub fn sync_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    format!("{prefix}-{}", hex::encode(bytes))
}

pub fn optional(value: Option<&str>) -> Option<String> {
    match value.map(str::trim) {
        Some("") | None => None,
        Some(value) => Some(value.to_owned()),
    }
}

fn truncate_bytes(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

pub fn sqlite_datetime_modifier(minutes: i64) -> Cow<'static, str> {
    Cow::Owned(format!("-{} minutes", minutes.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_matches_upstream_rules() {
        assert_eq!(project("  My--PROJECT__Name  "), "my-project_name");
        assert_eq!(scope("PERSONAL"), "personal");
        assert_eq!(scope("unknown"), "project");
        assert_eq!(
            topic_key(Some(" Architecture/Auth Model ")).as_deref(),
            Some("architecture/auth-model")
        );
    }

    /// The same memory, typed twice, hashes the same.
    ///
    /// `normalized_hash` folds whitespace and case before hashing, and that
    /// folding is the whole of duplicate detection: an agent re-saving a
    /// memory it reflowed, or capitalised differently, is the ordinary case
    /// this catches. A mutation run found the folding untested — remove it and
    /// every test still passed while `duplicate_count` stopped counting.
    #[test]
    fn the_duplicate_hash_ignores_whitespace_and_case_but_not_words() {
        let canonical = normalized_hash("The retry budget is per host");
        for same in [
            "the retry budget is per host",
            "The  retry   budget is per host",
            "The retry budget
is per host",
            "  THE RETRY BUDGET IS PER HOST  ",
        ] {
            assert_eq!(
                normalized_hash(same),
                canonical,
                "{same:?} is the same memory written differently"
            );
        }
        assert_ne!(
            normalized_hash("The retry budget is per request"),
            canonical,
            "a different claim is a different memory"
        );
    }

    #[test]
    fn private_content_is_redacted() {
        assert_eq!(
            strip_private("before <PRIVATE>secret\nvalue</private> after"),
            "before [REDACTED] after"
        );
    }

    #[test]
    fn fts_terms_are_quoted() {
        assert_eq!(fts_query("fix auth bug", false), "\"fix\" \"auth\" \"bug\"");
        assert_eq!(fts_query("fix auth", true), "\"fix\" OR \"auth\"");
    }

    #[test]
    fn synonyms_fold_onto_the_documented_types_and_unknown_ones_survive() {
        assert_eq!(kind("Bug"), "bugfix");
        assert_eq!(kind(" fix "), "bugfix");
        assert_eq!(kind("design"), "architecture");
        assert_eq!(kind("learning"), "discovery");
        assert_eq!(kind("CI"), "config");
        assert_eq!(kind("user"), "preference");

        // Already canonical, and left exactly as it is.
        for documented in [
            "bugfix",
            "decision",
            "architecture",
            "discovery",
            "pattern",
            "config",
            "preference",
        ] {
            assert_eq!(kind(documented), documented);
        }

        // Internal, and nothing folds onto it or away from it.
        assert_eq!(kind("session_summary"), "session_summary");

        // No documented type means these, so they keep their own word rather
        // than being forced into the nearest bucket. A real store holds them:
        // `implementation` names how something was built, and `feature` what
        // was built, and neither is a lie.
        assert_eq!(kind("implementation"), "implementation");
        assert_eq!(kind("feature"), "feature");

        // `manual` is the exception, because it is not a description at all —
        // it is the default `mem_save` leaves when the caller named no type,
        // and eighteen memories of a real store carry it while being invisible
        // to every typed search.
        assert_eq!(kind("manual"), "discovery");
    }

    #[test]
    fn topic_suggestion_matches_reference_families_and_fallbacks() {
        assert_eq!(
            suggest_topic_key("Architecture", "  Auth Model  ", "ignored"),
            "architecture/auth-model"
        );
        assert_eq!(
            suggest_topic_key(
                "bugfix",
                "",
                "Stop the session store panicking on a blank token"
            ),
            "bug/stop-the-session-store-panicking-on-a-blank"
        );
        assert_eq!(
            suggest_topic_key("manual", "", "Fix regression in auth login flow"),
            "bug/fix-regression-in-auth-login-flow"
        );
        assert_eq!(
            suggest_topic_key("decision", "decision", ""),
            "decision/general"
        );
        assert_eq!(suggest_topic_key("manual", "!!!", "..."), "topic/general");
    }

    /// Every language Leteo speaks has a heading a subagent can be captured by.
    ///
    /// The skill asks for `## Key Learnings:` in those words and the opening
    /// context tells the agent to write memories in the configured language, so
    /// an agent working in Portuguese ends with `## Aprendizados-chave` — the
    /// instruction followed, not ignored. English and Spanish were recognised,
    /// and Spanish is in the code because it happened.
    ///
    /// A miss is not a worse capture. The skill says what it is: without that
    /// section the subagent finishes, its context is discarded, and what it
    /// found is gone.
    /// A stored title is not cut shorter than any surface would show it.
    ///
    /// The passive capture cut at 60 — below the *median* of a real store'''s
    /// titles — and it was cutting what it had just been handed: a subagent'''s
    /// learning is one sentence and the title is a copy of it. Two in five
    /// arrived with the point taken off, then were shown under a bound that
    /// would have fitted them whole. Measured over 3,049 model-written
    /// one-liners: 40% cut at 60, 25% at 80, 13% at 100, 2% here.
    #[test]
    fn a_title_is_stored_at_the_length_it_is_shown_at() {
        assert_eq!(TITLE_CHARS, 140);
        // The sentence a subagent actually ends with, at the p90 of that store.
        let learning = "El indice de texto completo pierde las ediciones cuando un disparador desaparece, y nada lo comprueba";
        assert!(learning.chars().count() > 60 && learning.chars().count() <= TITLE_CHARS);
        assert_eq!(
            truncate_words(learning, TITLE_CHARS),
            learning,
            "a one-sentence learning has to survive whole"
        );
        assert!(
            truncate_words(learning, 60) != learning,
            "and the old bound is what it did not survive"
        );
        // Past the bound it is still cut, between words, with the marker
        // inside the budget.
        let long = "palabra ".repeat(60);
        let cut = truncate_words(&long, TITLE_CHARS);
        assert!(
            cut.chars().count() <= TITLE_CHARS,
            "{}",
            cut.chars().count()
        );
        assert!(cut.ends_with("..."), "{cut}");
    }

    /// A heading somebody adds later cannot turn a hook into a panic.
    ///
    /// Eight of the nine regexes in this file are constants: a mistyped one
    /// fails the build, which is the right place. This one is built at run time
    /// from a table, behind a `LazyLock` that the `subagent-stop` hook touches
    /// — so a heading with a bracket in it would not be a poor match, it would
    /// be a panic in a process whose whole promise is that it answers.
    ///
    /// The twelve headings there today are all letters and spaces, so the
    /// current table proves nothing about the next one. This feeds the builder
    /// what a thirteenth language might bring.
    #[test]
    fn no_heading_can_make_the_learning_regex_fail_to_compile() {
        let hostile: &[(&str, &[&str])] = &[
            (
                "xx",
                &["Key (Learnings)", "A [bracketed] one", "back\\slash"],
            ),
            ("yy", &["a+b*c?", "one|two", "^anchored$", "dot.star.*"]),
            ("zz", &["{2,3}", "(?i)inline", "trailing\\", "エラー", ""]),
        ];
        let pattern = heading_pattern_of(hostile);
        let built = Regex::new(&format!(r"(?im)^#{{2,3}}\s+(?:{pattern})s?:?\s*$"));
        assert!(
            built.is_ok(),
            "a heading table made the hook's regex invalid: {built:?}"
        );

        // And the escaping is escaping rather than dropping: a heading with a
        // metacharacter still matches itself and nothing else.
        let regex = built.unwrap();
        assert!(regex.is_match("## Key (Learnings)"));
        assert!(regex.is_match("## a+b*c?"));
        assert!(
            !regex.is_match("## Key Learnings"),
            "the parenthesis has to be a parenthesis, not a group"
        );

        // The real table compiles too, which is what the hook actually loads.
        assert!(
            Regex::new(&format!(
                r"(?im)^#{{2,3}}\s+(?:{})s?:?\s*$",
                learning_heading_pattern()
            ))
            .is_ok()
        );
    }

    #[test]
    fn a_subagent_is_captured_in_every_language_leteo_speaks() {
        let covered: Vec<&str> = LEARNING_HEADINGS.iter().map(|(code, _)| *code).collect();
        for language in crate::settings::Interface::ALL {
            assert!(
                covered.contains(&language.code()),
                "{} has no learning heading, so a subagent working in it is lost",
                language.as_str()
            );
        }
        assert_eq!(covered.len(), crate::settings::Interface::ALL.len());

        // Each spelling, in the shape a subagent actually ends with.
        for (code, headings) in LEARNING_HEADINGS {
            for heading in *headings {
                let text = format!(
                    "algo de trabajo\n\n## {heading}:\n1. Una cosa que merece recordarse y tiene bastantes palabras\n"
                );
                assert_eq!(
                    extract_learnings(&text).len(),
                    1,
                    "{code}: {heading:?} was not read as a learnings section"
                );
            }
        }

        // Accents are the first thing a terminal loses, so both spellings work.
        for heading in [
            "Lliçons Apreses",
            "Llicons Apreses",
            "Leçons Apprises",
            "Lecons Apprises",
        ] {
            let text = format!(
                "## {heading}\n- Una cosa que merece recordarse y tiene bastantes palabras\n"
            );
            assert_eq!(extract_learnings(&text).len(), 1, "{heading:?}");
        }

        // And a heading that is not one is still not one.
        assert!(
            extract_learnings("## Notes\n1. Una cosa que merece recordarse y tiene palabras\n")
                .is_empty()
        );
    }

    #[test]
    fn extracts_last_valid_learning_section_in_both_languages() {
        let text = r#"## Key Learnings:
1. This previous learning remains available as a fallback

## Aprendizajes Clave:
- **Usar** `transacciones` evita estados parciales durante escrituras complejas
- corto

## Next Steps
- this is not a learning
"#;
        assert_eq!(
            extract_learnings(text),
            vec!["Usar transacciones evita estados parciales durante escrituras complejas"]
        );

        let fallback = "## Key Learnings:\n1. This prior section has enough words to remain valid\n\n## Key Learnings:\n1. short\n";
        assert_eq!(
            extract_learnings(fallback),
            vec!["This prior section has enough words to remain valid"]
        );
    }

    /// A snippet between two learnings loses neither of them, and is not one.
    ///
    /// The extractor asks two things of the text and a fenced block answers
    /// both wrongly. A section ends at the next line opening with hashes and a
    /// space, which is how a shell comment is written: a subagent that
    /// followed the instruction exactly and put a snippet between its second
    /// and third learning had the section cut at the comment, so three
    /// numbered items came back as one and the hook reported "1 captured" as
    /// though it had worked. That is the same loss as a capture the store
    /// refused, with nothing at all to show for it.
    ///
    /// And the other way round: a numbered line inside a block was read as an
    /// item, so `1. export the variable first` was filed as something the
    /// subagent had learned rather than as the command it is.
    #[test]
    fn a_code_block_neither_ends_a_learning_section_nor_fills_one() {
        let entre_medias = "## Key Learnings:
             1. The index was left without its triggers and nothing said so
             
```sh
# the variable has to be exported first
export A=1
```

             2. The review window could never fire at all
             3. The hook answers empty when the store is busy
";
        assert_eq!(
            extract_learnings(entre_medias).len(),
            3,
            "{:?}",
            extract_learnings(entre_medias)
        );

        // What is inside the block is not a learning, numbered or not.
        let solo_codigo = "## Key Learnings:

```sh
             1. export the variable before anything else happens
             2. rebuild the whole index afterwards to be sure
```
";
        assert_eq!(extract_learnings(solo_codigo), Vec::<String>::new());

        // Y una valla sin cerrar dice justo eso: de aquí en adelante es código.
        let sin_cerrar = "## Key Learnings:
             1. The index was left without its triggers and nothing said so
             
```sh
2. this line is inside a block nobody closed
";
        assert_eq!(extract_learnings(sin_cerrar).len(), 1);

        // Masking preserves the positions byte for byte, which is what leaves
        // the sections carrying no block at all untouched.
        let con_acentos = "## Aprendizajes clave:
             1. La revisión quedó sin disparadores y nadie dijo nada
             
```rust
let ñ = 1;
```

             2. La ventana de relectura no podía dispararse nunca
";
        assert_eq!(
            extract_learnings(con_acentos),
            vec![
                "La revisión quedó sin disparadores y nadie dijo nada",
                "La ventana de relectura no podía dispararse nunca",
            ]
        );
    }

    /// A session is not titled with a line of code.
    ///
    /// The same mask, for the same reason one level over: a summary opening
    /// with a snippet was titled `cargo test --all --release`, which passes
    /// every test a headline has to pass and says nothing about the session.
    /// The title is weighted 5.0 in the ranking, so it is the field that
    /// decides whether the summary is found again — which is the whole reason
    /// this function exists.
    #[test]
    fn a_headline_is_prose_and_not_a_line_of_code() {
        let abre_con_codigo = "```sh
cargo test --all --release
```

             Arreglada la ventana de revisión que no disparaba nunca.
";
        assert_eq!(
            headline(abre_con_codigo, 120).as_deref(),
            Some("Arreglada la ventana de revisión que no disparaba nunca.")
        );

        let encabezado_y_bloque = "## Resumen

```rust
let indice = reconstruir(&tx)?;
```

             La sesión fue sobre reconstruir los índices de texto completo.
";
        assert_eq!(
            headline(encabezado_y_bloque, 120).as_deref(),
            Some("La sesión fue sobre reconstruir los índices de texto completo.")
        );

        // And a summary that is nothing but code has no headline to give, rather
        // than giving the first command that fits.
        let solo_codigo = "## Resumen

```sh
cargo test --all --release
```
";
        assert_eq!(headline(solo_codigo, 120), None);
    }

    /// The section both skills tell agents to write is one this can read.
    ///
    /// The skill asks subagents to end with a `## Key Learnings:` block, and
    /// the SubagentStop hook keeps what this extracts from it. Those are two
    /// halves of one promise held in two files, and nothing else would notice
    /// them drifting apart: change the header this accepts, or the shape the
    /// skill asks for, and capture goes quietly back to keeping nothing —
    /// which is exactly how it spent 3,530 memories capturing none.
    #[test]
    fn the_skill_asks_subagents_for_a_section_this_can_read() {
        for skill in [
            "plugin/claude-code/skills/memory/SKILL.md",
            "plugin/codex/skills/memory/SKILL.md",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(skill);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let (_, after) = text
                .split_once("## SUBAGENTS")
                .unwrap_or_else(|| panic!("{skill} has to tell subagents what to write"));
            // The first fenced block under that heading is the example, taken
            // verbatim rather than retyped here — a copy would go on passing
            // after the document it is quoting changed.
            let example = after
                .split_once("```")
                .and_then(|(_, rest)| rest.split_once("```"))
                .map(|(block, _)| block)
                .unwrap_or_else(|| panic!("{skill} shows no example block"));

            let learnings = extract_learnings(example);
            assert_eq!(
                learnings.len(),
                1,
                "the example {skill} shows has to be one this extracts from: {example:?} -> {learnings:?}"
            );
        }
    }

    #[test]
    fn every_synonym_folds_onto_a_kind_the_documents_promise() {
        // The fold table turns a word an agent reached for into one the skill
        // teaches. A target outside that list would file the memory under a
        // name the `type` filter is never asked for — the exact harm the table
        // exists to prevent, done by the table itself.
        for word in [
            "bug",
            "fix",
            "hotfix",
            "incident",
            "regression",
            "design",
            "adr",
            "refactor",
            "learning",
            "research",
            "investigation",
            "root_cause",
            "root-cause",
            "passive",
            "convention",
            "guideline",
            "rule",
            "setup",
            "infra",
            "infrastructure",
            "ci",
            "configuration",
            "feedback",
            "user",
            "preferences",
        ] {
            let folded = kind(word);
            assert!(
                crate::memory::rules::KINDS.contains(&folded.as_str()),
                "{word} folds onto {folded}, which no document mentions"
            );
            assert_ne!(
                folded, word,
                "{word} is listed as a synonym and folds nowhere"
            );
        }
    }

    #[test]
    fn a_documented_kind_is_left_exactly_as_it_arrived() {
        // Folding one of these would rename what an agent deliberately chose.
        for documented in crate::memory::rules::KINDS {
            assert_eq!(&kind(documented), documented);
        }
    }
}

#[cfg(test)]
mod headline_tests {
    use super::*;

    /// The shape a session summary actually has, from a real store.
    #[test]
    fn a_summary_is_titled_by_what_the_session_was_for() {
        let body = "## Goal\nAudit all open task-board tasks and correct stale states\n\n## Instructions\nsomething else";
        assert_eq!(
            headline(body, 72).as_deref(),
            Some("Audit all open task-board tasks and correct stale states")
        );
    }

    #[test]
    fn a_heading_names_the_section_and_never_the_session() {
        // `## Goal` is true of all 898 of them, so using it would have left
        // every title identical — which is the whole defect.
        assert_eq!(headline("## Goal\n", 72), None);
        assert_eq!(headline("# Session Summary\n---\n", 72), None);
    }

    #[test]
    fn a_line_too_short_to_be_a_headline_is_passed_over() {
        // A bare date or a leftover bullet. The name it already has is better.
        assert_eq!(headline("## Goal\n2026-08-02\n", 72), None);
        // Long enough by characters and still a label rather than a sentence.
        // This is what a client with no skill to follow writes, and the line
        // under it is the one worth having.
        assert_eq!(
            headline(
                "Session 2026-08-02\nRebuilt the chunk ordering after the change\n",
                72
            )
            .as_deref(),
            Some("Rebuilt the chunk ordering after the change")
        );
        assert_eq!(
            headline(
                "## Goal\nok\nRestore the deterministic ordering of chunks\n",
                72
            )
            .as_deref(),
            Some("Restore the deterministic ordering of chunks")
        );
    }

    #[test]
    fn a_long_line_is_cut_on_a_word_and_says_it_was_cut() {
        let body = "## Goal\nReconstruct the entire replication pipeline from the manifest downwards including every chunk\n";
        let cut = headline(body, 40).expect("there is a headline");
        assert!(cut.ends_with('…'), "{cut}");
        assert!(cut.chars().count() <= 41, "{cut}");
        assert!(!cut.contains("includ"), "cut mid-word: {cut}");
        assert_eq!(cut, "Reconstruct the entire replication…");
    }

    #[test]
    fn markdown_and_bullets_are_stripped_rather_than_titled() {
        assert_eq!(
            headline("## Goal\n- **Rebuild** the `observations_fts` index\n", 72).as_deref(),
            Some("Rebuild the observations_fts index")
        );
    }
}

#[cfg(test)]
mod skill_shape_tests {
    use super::*;

    /// The skill shows the shape; `headline` reads it. Two halves, two files.
    ///
    /// Every summary used to be called `Session summary: <project>`, so 507 of
    /// them shared a name and could not be found by it. The title now comes
    /// from the body — the first line that is neither blank nor a heading —
    /// which works because the skill asks for `## Goal` and then a line saying
    /// what the session was for.
    ///
    /// Change either side and the titles go quietly back to being identical:
    /// nothing fails, nothing warns, and the memories are simply unreachable
    /// again. So the example the skill prints is taken verbatim and run through
    /// the real extractor, the same way the subagent-learnings guard does.
    ///
    /// Deliberately not in `tools/guards.json`. That harness patches one file
    /// per case, and any single-file edit to a skill is caught first by the
    /// test holding the two bundles identical — which would make this look
    /// guarded when the case it exists for, the shape being changed in both,
    /// is the one a one-file mutation cannot express.
    #[test]
    fn the_shape_the_skill_asks_for_is_one_a_title_can_be_taken_from() {
        for skill in [
            "plugin/claude-code/skills/memory/SKILL.md",
            "plugin/codex/skills/memory/SKILL.md",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(skill);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            let (_, after) = text
                .split_once("## SESSION SUMMARY SHAPE")
                .unwrap_or_else(|| panic!("{skill} no longer shows a summary shape"));
            let example = after
                .split_once("```")
                .and_then(|(_, rest)| rest.split_once("```"))
                .map(|(block, _)| block)
                .unwrap_or_else(|| panic!("{skill} shows no example block"));

            let headline = headline(example, 72).unwrap_or_else(|| {
                panic!("no title can be taken from the shape {skill} asks for:\n{example}")
            });
            // The goal line, not the heading above it: `## Goal` is true of
            // every summary ever written, which is the defect this replaced.
            assert!(
                !headline.starts_with('#') && headline.len() >= MIN_HEADLINE_CHARS,
                "{skill}: {headline:?} is a heading, not a headline"
            );
        }
    }
}

/// The pure functions every search and every save runs through.
///
/// Nine of them had no test naming them at all — a sweep of the tree found 91
/// such functions, and this file held the largest cluster that is not a row
/// mapper or a screen. They are two lines each and they decide what a term is,
/// what gets stored and what a hook may look up; the length cap in
/// `truncate_content` had a real defect that shipped, and the word cap in
/// `prompt_terms` cost the prompt hint four points of accuracy.
#[cfg(test)]
mod plumbing {
    use super::*;

    /// A directory is not a project name, and the last segment is the answer.
    ///
    /// On a real store, 44 prompts and sessions were filed under paths — three
    /// distinct ones, and every last segment was a project that store actually
    /// held. Nothing found those rows again: every read narrows by project, so
    /// they sat in a project that existed nowhere else.
    /// An accent keeps its letter instead of breaking the word around it.
    ///
    /// A topic key keeps `[a-z0-9]` and turns everything else into a
    /// separator, so `decisión` became `decisi-n`: two tokens, neither of them
    /// the word. `topic_key` is weighted 3.0 in the ranking, so a key that
    /// spells its own subject wrong stops helping the search it is weighted
    /// for — and this store is written in Spanish.
    #[test]
    fn an_accent_in_a_topic_key_keeps_its_letter() {
        assert_eq!(
            topic_key(Some("Una decisión sobre SQLite")).as_deref(),
            Some("una-decision-sobre-sqlite")
        );
        assert_eq!(
            topic_key(Some("bug/año de compilación")).as_deref(),
            Some("bug/ano-de-compilacion")
        );
        // Every accent this store's two languages use, and the ones a French
        // or Portuguese title would bring.
        assert_eq!(
            topic_key(Some("áéíóú àèìòù äëïöü âêîôû ñç ã")).as_deref(),
            Some("aeiou-aeiou-aeiou-aeiou-nc-a")
        );
        // A dot survives, because a version number is the reason it is
        // allowed: 33 keys on a real store carry one. Everything else that is
        // not a letter, a digit or the family slash is still a separator.
        assert_eq!(
            topic_key(Some("Godot 4.7 — el runner")).as_deref(),
            Some("godot-4.7-el-runner")
        );
        assert_eq!(
            topic_key(Some("Arquitectura / decisión final")).as_deref(),
            Some("arquitectura/decision-final")
        );
    }

    #[test]
    fn a_project_name_that_is_a_directory_is_reduced_to_its_last_segment() {
        // The three real ones, spelled the way they were stored.
        assert_eq!(project(r"h:\repo\nas.archive"), "nas.archive");
        assert_eq!(project(r"h:\repo"), "repo");
        assert_eq!(
            project(r"\users\asanabrial\.agents\skills\task-board\"),
            "task-board"
        );
        // Forward slashes too, because a name written anywhere but Windows
        // arrives with those and one store holds both.
        assert_eq!(project("H:/REPO/leteo"), "leteo");

        // And what must not move: a name with no separator in it, including
        // the two that are merely wrong, and the two whose dots look like
        // separators and are not.
        for name in ["asanabrial", "eu", "nas.archive", "example-school.com"] {
            assert_eq!(project(name), name, "{name} is a name, not a path");
        }
    }

    #[test]
    fn a_project_is_narrowed_inside_the_index_and_its_name_is_quoted() {
        assert_eq!(
            fts_within_project("\"pool\" OR \"leak\"", "leteo").as_deref(),
            Some("(project : \"leteo\") AND (\"pool\" OR \"leak\")")
        );
        // A name the tokenizer splits is still one phrase, so it cannot match
        // a project that merely shares a word with it.
        assert_eq!(
            fts_within_project("\"x\"", "nas.archive").as_deref(),
            Some("(project : \"nas.archive\") AND (\"x\")")
        );
        // A quote closes the string it is written inside, so it is written
        // twice — otherwise a project could carry the rest of the query.
        assert_eq!(
            fts_within_project("\"x\"", "say \"hello\"").as_deref(),
            Some("(project : \"say \"\"hello\"\"\") AND (\"x\")")
        );
        // Nothing to index, nothing to narrow: an empty phrase is a syntax
        // error, and a query that fails is a hint that never speaks.
        assert_eq!(fts_within_project("\"x\"", "---"), None);
        assert_eq!(fts_within_project("\"x\"", ""), None);
    }

    #[test]
    fn a_word_asked_for_twice_is_searched_for_once() {
        // `"x" AND "x"` is `"x"`, and FTS5 does the work twice anyway: one word
        // repeated three hundred times measured 72.5 ms against 0.9 ms.
        assert_eq!(fts_terms("suelo suelo suelo"), vec!["\"suelo\""]);
        // Case is folded because the tokenizer folds it, and the first
        // spelling is what stays.
        assert_eq!(fts_terms("Suelo suelo SUELO"), vec!["\"Suelo\""]);
        // Order is the order it was written: the widened retry drops terms by
        // position, so a reordering here would rescue a different question.
        assert_eq!(
            fts_terms("bm25 suelo bm25 conflictos"),
            vec!["\"bm25\"", "\"suelo\"", "\"conflictos\""]
        );
        // And nothing is lost from a query with no repeats.
        assert_eq!(fts_terms("uno dos tres").len(), 3);
    }

    #[test]
    fn a_quote_in_a_question_is_escaped_rather_than_ending_the_phrase() {
        // An unescaped quote closes the FTS5 phrase and turns the rest of the
        // question into syntax. Engram had to fix exactly this.
        assert_eq!(fts_terms("say\"this"), vec!["\"say\"\"this\""]);
        // Wrapping quotes are the caller quoting a phrase, not part of a word.
        assert_eq!(fts_terms("\"exacto\""), vec!["\"exacto\""]);
    }

    #[test]
    fn only_the_word_still_being_typed_is_left_open() {
        // The last term is the one somebody has not finished, so `postgr` has
        // to reach `postgres`; the ones before it are finished words and
        // opening them would widen a search that was already narrowed.
        assert_eq!(fts_prefix_query("postgres pool"), "\"postgres\" \"pool\"*");
        assert_eq!(fts_prefix_query("pool"), "\"pool\"*");
        assert_eq!(fts_prefix_query("   "), "");
        // Not deduplicated, unlike `fts_terms`: folding the last word into an
        // earlier one would open the wrong term.
        assert_eq!(
            fts_prefix_query("postgres pool postgres"),
            "\"postgres\" \"pool\" \"postgres\"*"
        );
    }

    #[test]
    fn a_prompt_is_read_to_the_end_of_its_subject() {
        // Twelve words used to be the cut, and it landed in the middle of an
        // ordinary question — measured over 223 real prompts, moving it out to
        // thirty-two gained nine the right memory and lost none.
        let prompt = (0..40)
            .map(|i| format!("palabra{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        // Thirty-two written out, not `MAX_ANY_TERMS` compared against
        // itself: the number is the measurement, and reading it from the
        // constant would agree with whatever the constant became.
        assert_eq!(prompt_terms(&prompt).len(), 32);
        // Words under three letters carry nothing a search can use, repeats
        // spend a place for nothing, and a term is compared in one case.
        assert_eq!(prompt_terms("La memoria de LA MEMORIA"), vec!["memoria"]);
    }

    #[test]
    fn a_truncated_body_leaves_room_for_saying_so() {
        // The marker is part of the budget: appending it afterwards is how the
        // stored text ends up longer than the cap it was cut to.
        let long = "a".repeat(500);
        let (content, hash) = stored_content(&long, 100);
        assert!(
            content.len() <= 100,
            "cut to 100 bytes, and it is {}",
            content.len()
        );
        assert!(content.ends_with("... [truncated]"), "{content}");
        // The hash is of what was stored, not of what was offered — hashing
        // the raw text made a redacted memory look new on every pass.
        assert_eq!(hash, normalized_hash(&content));
        // A body inside the budget is untouched.
        let (short, _) = stored_content("cabe entero", 100);
        assert_eq!(short, "cabe entero");
    }

    /// Every disjunction is bounded, and no conjunction is.
    ///
    /// The bound was the hint's alone, and `search`'s last stage — which
    /// documents itself as running the hint's own rule — reached the same
    /// `OR` through the unbounded [`fts_terms`], as did `mode: any` from the
    /// tool and the command line. A pasted paragraph became one `OR` per word:
    /// over 200 real prompts, 52.6ms at the ninetieth percentile against 17.0,
    /// and it answered *fewer* of them, because with a hundred words `OR`ed
    /// together everything matches something and nothing clears a floor that
    /// is the median of what the query found.
    ///
    /// The conjunction stays unbounded on purpose: two hundred terms joined by
    /// `AND` match almost nothing and cost almost nothing to find out, and
    /// cutting them would answer a different question from the one quoted.
    #[test]
    fn a_disjunction_is_bounded_wherever_it_is_built() {
        let largo = (0..80)
            .map(|index| format!("palabra{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            fts_terms(&largo).len(),
            80,
            "todas las palabras son distintas"
        );

        let cualquiera = fts_query(&largo, true);
        assert_eq!(
            cualquiera.matches(" OR ").count(),
            MAX_ANY_TERMS - 1,
            "{cualquiera}"
        );
        // El primero se queda y el que cae del tope no: se corta por la cola,
        // que es el orden en que se escribieron.
        assert!(cualquiera.contains("\"palabra0\""), "{cualquiera}");
        assert!(!cualquiera.contains("\"palabra40\""), "{cualquiera}");

        // La otra puerta a la misma disyunción, la que usa la etapa cercana.
        let sueltas: Vec<String> = (0..80).map(|index| format!("palabra{index}")).collect();
        let cualesquiera = fts_any_of(&sueltas);
        assert_eq!(cualesquiera.matches(" OR ").count(), MAX_ANY_TERMS - 1);
        // Y las mismas que la otra puerta deja: por la cola, no por la cabeza.
        // Contar cuántas quedan no dice cuáles, y quedarse con las últimas
        // treinta y dos de un párrafo pegado es tirar la pregunta y buscar su
        // final.
        assert!(cualesquiera.starts_with("\"palabra0\""), "{cualesquiera}");
        assert!(!cualesquiera.contains("\"palabra40\""), "{cualesquiera}");

        // Y la conjunción, que no lleva tope.
        let todas = fts_query(&largo, false);
        assert_eq!(todas.matches('"').count(), 160, "{todas}");
        assert!(todas.contains("\"palabra79\""), "{todas}");
    }

    #[test]
    fn a_nul_byte_is_not_allowed_to_end_the_string() {
        // SQLite takes a NUL as the end of a text value, so anything after one
        // would be stored and searched as though it had never been written.
        assert_eq!(strip_nul("antes\0después"), "antesdespués");
        assert_eq!(fts_terms("antes\0después"), vec!["\"antesdespués\""]);
    }

    #[test]
    fn a_window_of_minutes_is_a_modifier_sqlite_understands() {
        assert_eq!(sqlite_datetime_modifier(30), "-30 minutes");
        // A window of nothing is a minute, and a negative one is too: this
        // builds the `datetime(?, ...)` bound that decides which prompt a
        // memory is attributed to, and `-0 minutes` would attribute only what
        // arrived in the same second while `+5 minutes` would reach into the
        // future for it.
        assert_eq!(sqlite_datetime_modifier(0), "-1 minutes");
        assert_eq!(sqlite_datetime_modifier(-5), "-1 minutes");
    }
}

#[cfg(test)]
mod truncation_tests {
    use super::*;

    /// A shortened line ends on a word, and inside the budget it was given.
    ///
    /// Two copies of this cut wherever the count ran out — `al cerrar el
    /// clie...` — and appended the marker afterwards, so a caller asking for
    /// sixty characters got sixty-three. Over the 2,068 one-line sentences
    /// longer than sixty characters in a real store, 73.7% land inside a word.
    ///
    /// It matters most where the result is stored rather than rendered: the
    /// title `passive_capture` builds is a row, and it is what search weights
    /// five times the body.
    #[test]
    fn a_shortened_line_ends_on_a_word_and_inside_its_budget() {
        let learning = "El pool de conexiones no se devolvia nunca al cerrar el cliente";
        let short = truncate_words(learning, 60);
        assert!(
            short.chars().count() <= 60,
            "the marker comes out of the budget: {short:?} is {} characters",
            short.chars().count()
        );
        assert!(short.ends_with("..."), "{short:?}");
        let words = short.trim_end_matches("...");
        assert!(
            learning
                .split_whitespace()
                .any(|word| word == words.split_whitespace().last().unwrap()),
            "the last word survived whole: {short:?}"
        );
        assert!(!short.contains("clie..."), "cut inside a word: {short:?}");

        // What fits is returned untouched, with its whitespace folded.
        assert_eq!(truncate_words("  two   words  ", 60), "two words");

        // A single word with no boundary to find is cut where the budget ends,
        // because returning nothing would be worse than returning something.
        let unbroken = "a".repeat(80);
        let cut = truncate_words(&unbroken, 20);
        assert_eq!(cut.chars().count(), 20, "{cut:?}");
        assert!(cut.ends_with("..."));

        // And a budget with no room for the marker still returns text.
        assert_eq!(truncate_words("abcdef", 3).chars().count(), 3);
    }
}

#[cfg(test)]
mod topic_key_tests {
    use super::*;

    /// A key handed back is the key, not a title to build another one out of.
    ///
    /// The suggester built `{family}/{whole title}` from whatever it was given,
    /// so feeding it its own answer nested one more level every time:
    /// `search/coste-de-la-etapa-ensanchada` came back
    /// `topic/search/coste-de-la-etapa-ensanchada`, then `topic/topic/…`,
    /// without bound. It did it to a family it recognised too, because the
    /// guard it had strips a family joined by a hyphen and this one arrives
    /// joined by a slash.
    ///
    /// It matters beyond the shape. A topic key is how a memory is revised
    /// rather than written again, and search's exact branch looks one up the
    /// way it was stored: a key that gained a level matches nothing, so the
    /// revision becomes an insert.
    #[test]
    fn suggesting_a_key_from_a_key_gives_back_the_same_key() {
        let key = suggest_topic_key("discovery", "search/coste-de-la-etapa-ensanchada", "body");
        assert_eq!(key, "search/coste-de-la-etapa-ensanchada");
        assert_eq!(
            suggest_topic_key("discovery", &key, "body"),
            key,
            "idempotent"
        );

        // Including one whose family the suggester would have inferred anyway,
        // which is where it used to double the family instead of nesting.
        assert_eq!(
            suggest_topic_key("architecture", "architecture/wizard-split", "body"),
            "architecture/wizard-split"
        );

        // A real title still gets a key built for it, family and all.
        assert_eq!(
            suggest_topic_key("bugfix", "Fixed the connection pool leak", "body"),
            "bug/fixed-the-connection-pool-leak"
        );

        // And a title that merely holds a slash is a title: it is not what the
        // store would have kept, so it falls through and is built from.
        let from_a_path = suggest_topic_key("discovery", "CROSS JOIN in src/store/search.rs", "b");
        assert_ne!(from_a_path, "CROSS JOIN in src/store/search.rs");
        assert!(from_a_path.contains("cross-join-in-"), "{from_a_path}");
    }
}

#[cfg(test)]
mod idempotence_tests {
    use super::*;

    /// Every normaliser gives the same answer when handed its own answer.
    ///
    /// This is the property a normaliser is *for*: text arrives by more than one
    /// route — typed, replicated, restored from an export, read back and written
    /// again — and a rule that changes its mind on the second pass turns one
    /// value into two. `suggest_topic_key` failed exactly this and nested a
    /// level every time it saw its own output.
    ///
    /// The inputs are the shapes that actually reach these: accents, mixed case,
    /// paths, newlines, private markers, the marker a truncation leaves behind.
    #[test]
    fn a_normaliser_handed_its_own_answer_repeats_it() {
        let long = "a".repeat(200);
        let awkward: Vec<&str> = vec![
            "",
            " ",
            "H:/REPO/leteo",
            r"H:\REPO\Leteo\",
            "Leteo",
            "my  project",
            "a--b__c",
            "Architecture/Wizard-Split",
            "título con acentos y ñ",
            "line one\nline two",
            "<private>secret</private> rest",
            "... [truncated]",
            "bug",
            "PROJECT",
            "señal/de-prueba",
            &long,
        ];

        for raw in awkward {
            let once = project(raw);
            assert_eq!(project(&once), once, "project({raw:?})");

            let once = scope(raw);
            assert_eq!(scope(once), once, "scope({raw:?})");

            let once = kind(raw);
            assert_eq!(kind(&once), once, "kind({raw:?})");

            let once = one_line(raw).to_string();
            assert_eq!(one_line(&once), once, "one_line({raw:?})");

            let once = strip_private(raw);
            assert_eq!(strip_private(&once), once, "strip_private({raw:?})");

            let once = prompt_core(raw);
            assert_eq!(prompt_core(&once), once, "prompt_core({raw:?})");

            if let Some(once) = topic_key(Some(raw)) {
                assert_eq!(
                    topic_key(Some(&once)),
                    Some(once.clone()),
                    "topic_key({raw:?})"
                );
            }

            for budget in [16, 40, 400] {
                let once = truncate_content(raw.to_string(), budget);
                assert_eq!(
                    truncate_content(once.clone(), budget),
                    once,
                    "truncate_content({raw:?}, {budget})"
                );
                assert!(once.len() <= budget.max(raw.len()), "{once:?}");

                let once = truncate_words(raw, budget);
                assert_eq!(
                    truncate_words(&once, budget),
                    once,
                    "truncate_words({raw:?}, {budget})"
                );
            }

            let once = suggest_topic_key("discovery", raw, "some body text");
            assert_eq!(
                suggest_topic_key("discovery", &once, "some body text"),
                once,
                "suggest_topic_key({raw:?})"
            );
        }
    }
    /// No cut ever lands inside a character, at any offset, on any path.
    ///
    /// Every bound this crate publishes is a place a slice could split a `ñ` or an
    /// emoji, and in Rust that is a panic rather than a mojibake — in the MCP
    /// server, which is a process an agent is talking to. The store this was
    /// written against is mostly Spanish and the hooks print cats.
    ///
    /// Driven at every offset either side of each bound rather than at the bound,
    /// because the interesting case is not "a multi-byte character somewhere" but
    /// "a multi-byte character starting one byte before the cut". A test that puts
    /// one in the middle of the text proves nothing about the edge.
    #[test]
    fn no_bound_ever_cuts_a_character_in_half() {
        // The two byte-counted bounds and the four counted in characters.
        let bounds = [
            crate::mcp::PREVIEW_BYTES,
            crate::memory::normalize::TITLE_CHARS,
            crate::recall::CONTEXT_PREVIEW_CHARS,
            crate::recall::SESSION_LINE_CHARS,
            crate::recall::PROMPT_LINE_CHARS,
        ];
        assert!(bounds.iter().all(|bound| *bound > 100), "{bounds:?}");

        for bound in bounds {
            // Every position rather than the ones beside the bound. Each of
            // these subtracts something of its own before it cuts —
            // `truncate_content` takes the fifteen bytes of its marker out of
            // the budget first — so a sweep around the nominal bound can miss
            // the real cut entirely. It did: the first version of this put the
            // character near `bound` and passed with the boundary walk deleted.
            for at in 0..=bound + 8 {
                let filler = "a".repeat(at);
                // Four bytes, one character: the widest thing a cut can land in.
                let text = format!("{filler}🐈{}", "b".repeat(64));
                let offset = at as i64 - bound as i64;

                let cut = crate::memory::normalize::truncate_content(text.clone(), bound);
                assert!(
                    std::str::from_utf8(cut.as_bytes()).is_ok(),
                    "truncate_content at {bound}{offset:+} produced something that is not text"
                );
                assert!(
                    !cut.contains('\u{fffd}'),
                    "truncate_content at {bound}{offset:+} produced a replacement character"
                );

                let words = crate::memory::normalize::truncate_words(&text, bound);
                assert!(
                    words.chars().count() <= bound,
                    "truncate_words at {bound}{offset:+} kept {} characters",
                    words.chars().count()
                );
                assert!(!words.contains('\u{fffd}'), "{bound}{offset:+}");

                // And the whole write-then-read path, which is where a panic would
                // actually reach somebody: the body is stored, cut for a preview,
                // and rendered into the block a hook prints.
                let one_line = crate::memory::normalize::one_line(&text);
                assert!(!one_line.contains('\u{fffd}'), "{bound}{offset:+}");
            }
        }
    }
}
