//! How Leteo writes and reads times.
//!
//! Every timestamp in the database is a SQLite `DATETIME` string in UTC, and
//! the format had been spelled out at each of the nine places that produced or
//! consumed one. Duplication is the smaller problem: the parsers had already
//! drifted apart. `hooks` accepted RFC 3339 as well, so a row written by one
//! path parsed there and came back as `None` in `model` and in the autosync
//! loop — which read as "no deadline" rather than as an error, and quietly
//! skipped work.
//!
//! One format constant and one tolerant parser, used everywhere.

use chrono::NaiveDateTime;

pub const FORMAT: &str = "%Y-%m-%d %H:%M:%S";

pub fn format(value: NaiveDateTime) -> String {
    value.format(FORMAT).to_string()
}

pub fn now() -> String {
    format(chrono::Utc::now().naive_utc())
}

/// Reads a stored timestamp.
///
/// RFC 3339 is accepted alongside the native format because rows have arrived
/// from Engram adoption and from the cloud, and both have written it that way.
/// Rejecting those would lose real data for a difference in spelling.
pub fn parse(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    NaiveDateTime::parse_from_str(value, FORMAT)
        .ok()
        .or_else(|| {
            chrono::DateTime::parse_from_rfc3339(value)
                .ok()
                .map(|timestamp| timestamp.naive_utc())
        })
}

/// The shapes a date argument may be written in.
///
/// Named here because it is also what a refusal says, and a person learns the
/// formats from the error or from nowhere. SQLite's `datetime()` reads more
/// than these — `now`, `-1 day` and its other modifiers — which is the trap:
/// `datetime('yesterday')` is NULL, so a filter compares NULL and the command
/// answers an empty page as a success.
pub const ARGUMENT_FORMATS: &str = "`YYYY-MM-DD HH:MM:SS`, an RFC 3339 timestamp, or `YYYY-MM-DD`";

/// A date argument in none of the shapes [`ARGUMENT_FORMATS`] names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTimestamp {
    value: String,
}

impl std::fmt::Display for InvalidTimestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} is not a date Leteo can read; a date argument takes {}",
            self.value, ARGUMENT_FORMATS
        )
    }
}

impl std::error::Error for InvalidTimestamp {}

/// Reads a date somebody typed, in the one place every argument is read.
///
/// The answer is the native format, so a filter compares one spelling whether
/// the argument arrived as a date, an instant with an offset, or a stored row's
/// timestamp. A date with no time is midnight UTC, which is what SQLite's own
/// `datetime('2026-11-26')` means and what a person writing a day is asking
/// for.
pub fn parse_argument(value: &str) -> Result<String, InvalidTimestamp> {
    let value = value.trim();
    let parsed = parse(value).or_else(|| {
        chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .ok()
            .and_then(|date| date.and_hms_opt(0, 0, 0))
    });
    parsed.map(format).ok_or_else(|| InvalidTimestamp {
        value: value.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_formatted_time_reads_back_unchanged() {
        let value = NaiveDateTime::parse_from_str("2026-07-29 14:21:42", FORMAT).unwrap();
        assert_eq!(format(value), "2026-07-29 14:21:42");
        assert_eq!(parse(&format(value)), Some(value));
    }

    #[test]
    fn rfc3339_is_accepted_and_converted_to_utc() {
        // Rows adopted from Engram and rows arriving from the cloud have both
        // been written this way. Reading them as absent would silently drop
        // review deadlines and sync cursors.
        let parsed = parse("2026-07-29T16:21:42+02:00").expect("RFC 3339 must be accepted");
        assert_eq!(format(parsed), "2026-07-29 14:21:42");
    }

    #[test]
    fn surrounding_space_does_not_defeat_it() {
        assert!(parse("  2026-07-29 14:21:42  ").is_some());
    }

    #[test]
    fn a_date_argument_becomes_the_native_format() {
        assert_eq!(
            parse_argument("2026-07-29 14:21:42").unwrap(),
            "2026-07-29 14:21:42"
        );
        assert_eq!(
            parse_argument("2026-07-29T16:21:42+02:00").unwrap(),
            "2026-07-29 14:21:42",
            "an offset is converted to UTC, as everywhere else"
        );
        assert_eq!(
            parse_argument(" 2026-07-29 ").unwrap(),
            "2026-07-29 00:00:00",
            "a day with no time is its midnight"
        );
    }

    #[test]
    fn an_unreadable_date_argument_names_the_formats() {
        // Everything SQLite would have answered NULL for, plus two shapes a
        // looser parser is tempted by: a date with no day and an impossible
        // one, which `datetime` also refuses but by returning NULL rather than
        // an error.
        for value in [
            "",
            "   ",
            "yesterday",
            "now",
            "-1 day",
            "2026-02-30",
            "2026-07",
        ] {
            let refused = parse_argument(value).unwrap_err();
            let said = refused.to_string();
            assert!(said.contains(ARGUMENT_FORMATS), "{value:?} said {said}");
            if !value.trim().is_empty() {
                assert!(said.contains(value.trim()), "{value:?} said {said}");
            }
        }
    }

    #[test]
    fn nonsense_is_none_rather_than_a_wrong_time() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("yesterday"), None);
        assert_eq!(parse("2026-13-45 99:99:99"), None);
    }
}
