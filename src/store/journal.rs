//! What a delete leaves in the journal it shares with every peer.
//!
//! `sync_mutations.payload` holds the whole of every write, sent or unsent: a
//! memory's title and body, a prompt's words. Deleting the row does not delete
//! that, and a journal is a queue rather than a log — the peer that has not
//! pulled a mutation still needs one at that position, and the sequence the push
//! cursor and `last_enqueued_seq` refer to must not develop holes. So the rows
//! are rewritten to the fields a peer needs to place the mutation, and the text
//! is dropped.
//!
//! The keys that survive are the caller's, because the two entities need
//! different ones; the cut is here so there is one implementation of it. A
//! payload that is not a JSON object — including one the journal wrapped as a
//! JSON string, which the apply path also accepts — is replaced by the
//! identifier alone instead of being left in place: an unreadable payload is
//! exactly the one nobody can show holds no text.

use rusqlite::{Transaction, params_from_iter};

use crate::store::StoreError;

/// Rewrites every journalled row of these entities to `key`'s kept fields.
///
/// `entities` is the set of `sync_mutations.entity` values whose payloads carry
/// this record's text — an observation has two, because its versions are queued
/// as rows of their own.
pub(super) fn redact_journal_tx(
    tx: &Transaction<'_>,
    entities: &[&str],
    entity_key: &str,
    kept: &[&str],
) -> Result<(), StoreError> {
    if entity_key.is_empty() {
        return Ok(());
    }
    let rows: Vec<(i64, String)> = {
        let placeholders = vec!["?"; entities.len()].join(", ");
        let mut statement = tx.prepare(&format!(
            "SELECT seq, payload FROM sync_mutations
              WHERE entity_key = ?1 AND entity IN ({placeholders})"
        ))?;
        let mut values = vec![entity_key.to_owned()];
        values.extend(entities.iter().map(|entity| (*entity).to_owned()));
        statement
            .query_map(params_from_iter(values), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?
            .collect::<Result<_, _>>()?
    };
    for (seq, payload) in rows {
        let redacted = redact_payload(&payload, entity_key, kept);
        if redacted != payload {
            tx.execute(
                "UPDATE sync_mutations SET payload = ?1 WHERE seq = ?2",
                rusqlite::params![redacted, seq],
            )?;
        }
    }
    Ok(())
}

/// A payload cut down to `kept`, whatever it held before.
fn redact_payload(payload: &str, entity_key: &str, kept: &[&str]) -> String {
    let mut value: serde_json::Value = serde_json::from_str(payload.trim()).unwrap_or_default();
    if let serde_json::Value::String(inner) = &value {
        value = serde_json::from_str(inner).unwrap_or_default();
    }
    let mut kept_fields = serde_json::Map::new();
    if let serde_json::Value::Object(object) = value {
        for (key, field) in object {
            if kept.contains(&key.as_str()) {
                kept_fields.insert(key, field);
            }
        }
    }
    kept_fields
        .entry("sync_id")
        .or_insert_with(|| serde_json::Value::String(entity_key.to_owned()));
    serde_json::Value::Object(kept_fields).to_string()
}
