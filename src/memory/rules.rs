//! What Leteo believes about a memory, with no database in sight.
//!
//! These are the rules — what a valid memory looks like, which type is which,
//! when a decision goes stale, what two memories may claim about each other.
//! They lived inside the SQLite adapter, mixed in with the statements that
//! stored their results, and that had a measurable cost: a memory could be
//! written two ways, locally or by replication, and the two paths had drifted
//! apart on six of eight of these rules without anything noticing.
//!
//! Nothing here opens a connection or builds a statement. That is the whole
//! point, and the tests below hold it to it: every rule is exercised without a
//! store.
//!
//! It is not a ceremony of ports and adapters. Leteo has one storage engine and
//! is not looking for a second — SQLite *is* the product. What it needed was a
//! single place that decides what a memory is, so that the places which merely
//! persist one cannot each decide differently.

/// What one memory may claim about another.
///
/// The vocabulary sat in the storage module, next to the statements that wrote
/// it. It is not a storage detail: it is the set of claims Leteo understands,
/// and the store re-exports these so nothing outside had to move.
pub const RELATION_RELATED: &str = "related";
pub const RELATION_COMPATIBLE: &str = "compatible";
pub const RELATION_SCOPED: &str = "scoped";
pub const RELATION_CONFLICTS_WITH: &str = "conflicts_with";
pub const RELATION_SUPERSEDES: &str = "supersedes";
pub const RELATION_NOT_CONFLICT: &str = "not_conflict";

/// The kinds Leteo asks for, in the order the skill teaches them.
///
/// Not a validation list. A kind outside it is stored verbatim on purpose —
/// [`crate::memory::normalize::kind`] explains why, and a real store holds
/// `implementation` and `feature` because of it. This is what the documents
/// promise: the `type` field of `mem_save`, and the skill every agent reads.
///
/// It lived only in that prose, in two files, with nothing tying either to the
/// code. It is here so the fold table below can be held to it: a synonym that
/// folded onto a word the documents never mention would quietly file a memory
/// where nobody looks for it.
pub const KINDS: &[&str] = &[
    "bugfix",
    "decision",
    "policy",
    "architecture",
    "discovery",
    "pattern",
    "config",
    "preference",
];

/// Whether a search narrowed by type can ever return a memory of this kind.
///
/// [`KINDS`] is what an agent is taught to write and what a filter is asked
/// for. `session_summary` is not in it and is not a mistake: nothing outside
/// Leteo writes one, and the tools that list summaries ask for them by name.
/// Anything else outside the list is a memory filed where nobody looks — see
/// `UNFILED_KIND_HINT`.
pub fn is_searchable_kind(kind: &str) -> bool {
    KINDS.contains(&kind) || kind == crate::memory::model::SESSION_SUMMARY
}

/// The kinds that go stale, and how long each stays trustworthy.
///
/// Only kinds that go stale get a window. A discovery about how a parser
/// behaves is as true in a year as it is today; a decision may not be, and a
/// stated preference is the shortest-lived of the three.
///
/// A table rather than arms in the function below, so that something can walk
/// it. `policy` once had a twelve-month window while being missing from
/// [`KINDS`] — so no agent could write one, the window never fired and never
/// could, and a store of three and a half thousand memories held not a single
/// policy. The test that exists to catch that compared against a third
/// hand-written copy of these three names, which meant a *new* kind given a
/// window and left out of `KINDS` would repeat the whole thing untouched.
/// Listed once, the check below reads the same list the behaviour does.
pub(crate) const REVIEW_WINDOWS: &[(&str, u32)] =
    &[("decision", 6), ("policy", 12), ("preference", 3)];

/// When a memory of this kind wants rereading, counted from a moment.
///
/// One function because there were three, and they did not agree. Saving a
/// memory and marking one reviewed both counted calendar months; rewinding the
/// clock when a memory's type changed counted months of thirty days, and a
/// migration written from that one inherited it. Four days apart on a
/// six-month window — nothing anybody would notice, and exactly the shape that
/// has cost this codebase real bugs: `REVIEW_WINDOWS` itself was consolidated
/// after a third hand-written copy of these names let `policy` keep a window
/// nothing could fire.
///
/// Calendar months, because that is what the rule says in words: a decision is
/// good for six months, not for a hundred and eighty days.
///
/// `None` for a kind that does not go stale, which is most of them.
pub fn review_after(kind: &str, from: chrono::NaiveDateTime) -> Option<chrono::NaiveDateTime> {
    let months = review_months(kind)?;
    Some(
        from.checked_add_months(chrono::Months::new(months))
            .unwrap_or(from),
    )
}

pub fn review_months(kind: &str) -> Option<u32> {
    REVIEW_WINDOWS
        .iter()
        .find(|(known, _)| *known == kind)
        .map(|(_, months)| *months)
}

/// Every claim one memory may make about another.
///
/// A list rather than the arms of a `match`, because three places wanted to
/// read it and each wrote its own copy: the check below, the test that holds
/// the check, and the descriptions two tools ship to every agent. The refusal
/// could not name them at all — it said "invalid relation verb: X" and left the
/// caller to guess, while `doctor` refusing a check code has listed the valid
/// ones since it was written.
///
/// This is the fourth vocabulary to be consolidated for the same reason, after
/// the types, the review windows and the hook event names. A hand-written
/// second copy is how `policy` kept a review window nothing could fire.
pub const RELATION_VERBS: &[&str] = &[
    RELATION_RELATED,
    RELATION_COMPATIBLE,
    RELATION_SCOPED,
    RELATION_CONFLICTS_WITH,
    RELATION_SUPERSEDES,
    RELATION_NOT_CONFLICT,
];

pub fn is_relation_verb(relation: &str) -> bool {
    RELATION_VERBS.contains(&relation)
}

pub fn is_confidence(confidence: f64) -> bool {
    (0.0..=1.0).contains(&confidence)
}

/// Why a memory may be refused outright.
///
/// Refusing is not the same as normalising, and the difference decides where
/// each belongs. Normalising is safe on any path, so it happens once in
/// [`crate::memory::normalize::fields`] and every writer goes through it. Refusing is
/// not: a caller can be told to add a title and try again, and a peer
/// replicating a memory written years ago cannot — refusing that one loses it.
/// So rejection stays at the local door, and this is that door.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The dashboard lists titles, the session context is an index of titles,
    /// and full-text search weights the title five times the body. A memory
    /// without one is stored, counted, and invisible. Eighty arrived in a real
    /// store this way before anything noticed.
    NoTitle,
    /// A memory with nothing in it records that something happened and not what.
    NoContent,
    /// The same, for the question rather than the answer.
    NoPrompt,
}

impl Refusal {
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoTitle => "title is required: a memory with no title cannot be found again",
            Self::NoContent => "content is required: a memory with no body records nothing",
            Self::NoPrompt => "content is required: a prompt with no words records nothing",
        }
    }
}

/// Checks a memory a caller is asking us to store.
pub fn refuse(title: &str, content: &str) -> Option<Refusal> {
    if title.trim().is_empty() {
        return Some(Refusal::NoTitle);
    }
    if content.trim().is_empty() {
        return Some(Refusal::NoContent);
    }
    None
}

/// Checks a prompt a caller is asking us to store.
///
/// The same door, one field wide. It did not exist here, and a real store held
/// eleven prompts recording that somebody had asked something and not what —
/// `mem_save_prompt` reported success for an empty string, for spaces, and for
/// a newline and a tab.
///
/// It is worth more than a wasted row. A prompt is what a memory is linked to
/// when it records the question it answers, so an empty one is a link to
/// nothing; and it takes one of the ten places the opening context keeps for
/// saying what somebody has been asking about.
pub fn refuse_prompt(content: &str) -> Option<Refusal> {
    content.trim().is_empty().then_some(Refusal::NoPrompt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The eleven prompts in a real store that recorded somebody had asked
    /// something and not what. Whitespace is the whole point: an empty string
    /// was never the shape that got through — a newline, a tab and a run of
    /// spaces were, and each is a link to nothing occupying one of the ten
    /// places the opening block keeps for what somebody has been asking about.
    #[test]
    fn a_prompt_made_of_nothing_is_refused_however_it_is_spelled() {
        for blank in ["", " ", "   ", "\n", "\t", "\r\n", " \t\n ", "\u{a0}"] {
            assert_eq!(
                refuse_prompt(blank),
                Some(Refusal::NoPrompt),
                "{blank:?} was stored"
            );
        }
        for real in ["?", "por que", "  por que  "] {
            assert_eq!(refuse_prompt(real), None, "{real:?} was refused");
        }
    }

    #[test]
    fn only_kinds_that_go_stale_ask_to_be_reread() {
        assert_eq!(review_months("decision"), Some(6));
        assert_eq!(review_months("policy"), Some(12));
        assert_eq!(review_months("preference"), Some(3));
        // A discovery is as true in a year as it is today.
        assert_eq!(review_months("discovery"), None);
        assert_eq!(review_months("bugfix"), None);
        assert_eq!(review_months("architecture"), None);
    }

    #[test]
    fn a_kind_that_goes_stale_has_to_be_one_somebody_can_write() {
        // `policy` had a twelve-month window and the skill told agents to ask
        // before superseding one — and it was missing from the list the `type`
        // field offers, so no agent could ever create one. The window never
        // fired and never could; a real store of three and a half thousand
        // memories held not a single policy.
        //
        // Two places believed in the kind and the third did not offer it.
        //
        // Read off `REVIEW_WINDOWS` rather than off a list written here. The
        // version before this walked `KINDS` discarding the result — proving
        // nothing at all — and then checked three names it had typed out
        // itself, so a *new* kind given a window and left out of `KINDS` would
        // have repeated the original bug with this test still green.
        for (kind, months) in REVIEW_WINDOWS {
            assert!(
                KINDS.contains(kind),
                "{kind} goes stale after {months} months and is not a kind anybody can write"
            );
        }
        // And the window is what the table says, so the lookup cannot quietly
        // stop finding entries.
        assert_eq!(review_months("policy"), Some(12));
        assert_eq!(review_months("nonsense"), None);
    }

    #[test]
    fn a_relation_verb_is_one_of_the_six_we_understand() {
        // Walked rather than copied: a fourth spelling of these six is how a
        // verb gets added to the list and refused by the check.
        assert_eq!(RELATION_VERBS.len(), 6, "{RELATION_VERBS:?}");
        for verb in RELATION_VERBS {
            assert!(is_relation_verb(verb), "{verb} should be a verb");
        }
        assert!(!is_relation_verb("supersedes_maybe"));
        assert!(!is_relation_verb(""));
        assert!(!is_relation_verb("SUPERSEDES"), "verbs are not case-folded");
    }

    #[test]
    fn confidence_is_a_probability_and_the_ends_are_allowed() {
        assert!(is_confidence(0.0));
        assert!(is_confidence(1.0));
        assert!(is_confidence(0.5));
        assert!(!is_confidence(-0.1));
        assert!(!is_confidence(1.1));
        assert!(!is_confidence(f64::NAN), "a range check rejects NaN");
    }

    #[test]
    fn a_memory_needs_a_title_and_a_body_to_be_worth_storing() {
        assert_eq!(refuse("", "body"), Some(Refusal::NoTitle));
        assert_eq!(refuse("   \n\t ", "body"), Some(Refusal::NoTitle));
        assert_eq!(refuse("title", ""), Some(Refusal::NoContent));
        assert_eq!(refuse("title", "  "), Some(Refusal::NoContent));
        assert_eq!(refuse("title", "body"), None);
        // The title is checked first, because it is the one a caller most often
        // forgets and the message should name it.
        assert_eq!(refuse("", ""), Some(Refusal::NoTitle));
    }

    #[test]
    fn every_refusal_says_why_rather_than_just_no() {
        for refusal in [Refusal::NoTitle, Refusal::NoContent] {
            let message = refusal.message();
            assert!(message.contains("required"), "{message}");
            assert!(
                message.len() > 40,
                "a refusal explains the consequence: {message}"
            );
        }
    }
}
