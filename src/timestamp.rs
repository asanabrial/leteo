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

/// The shape of a timestamp as SQLite stores it.
pub const FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// Renders a time the way the database expects it.
pub fn format(value: NaiveDateTime) -> String {
    value.format(FORMAT).to_string()
}

/// The current UTC time, ready to store.
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
    fn nonsense_is_none_rather_than_a_wrong_time() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("yesterday"), None);
        assert_eq!(parse("2026-13-45 99:99:99"), None);
    }
}
