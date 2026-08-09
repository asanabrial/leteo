//! Turning a SQLite row into one of our types, and back out again.

use super::*;

// One function per question, not one per handle. `Transaction` dereferences to
// `Connection`, so `&tx` coerces at the call site and the transaction-only
// twins these replaced — three functions carrying byte-identical SQL — were
// paying for a type difference that Rust already erases.

pub(super) fn get_session_row(connection: &Connection, id: &str) -> Result<Session, StoreError> {
    connection
        .query_row(
            "SELECT id, project, directory, started_at, ended_at, summary FROM sessions WHERE id = ?1",
            [id],
            map_session,
        )
        .optional()?
        .ok_or_else(|| StoreError::SessionNotFound(id.to_owned()))
}

/// Which of the two an absent live row was: deleted, or never there.
///
/// Asked only on the way to an error, so an ordinary read pays nothing for it.
/// Five doors used to answer both with `observation_not_found` while
/// `mem_get_observation` handed the same id back with `state: deleted`, so the
/// store knew and said it in one place out of six.
pub(super) fn deleted_or_missing(connection: &Connection, id: i64) -> StoreError {
    match connection.query_row(
        "SELECT deleted_at FROM observations WHERE id = ?1",
        [id],
        |row| row.get::<_, Option<String>>(0),
    ) {
        Ok(Some(deleted_at)) => StoreError::ObservationDeleted { id, deleted_at },
        _ => StoreError::ObservationNotFound(id),
    }
}

pub(super) fn get_active_observation(
    connection: &Connection,
    id: i64,
) -> Result<Observation, StoreError> {
    connection
        .query_row(
            &format!(
                "SELECT {OBSERVATION_COLUMNS} FROM observations
                 WHERE id = ?1 AND deleted_at IS NULL"
            ),
            [id],
            map_observation,
        )
        .optional()?
        .map_or_else(
            || {
                // Only on the way to an error, so the ordinary hit pays
                // nothing: ask whether the row is there at all, and say which
                // of the two happened. "Not found" for a memory that is sitting
                // in the table with a tombstone on it sends whoever asked to
                // doubt their id.
                Err(deleted_or_missing(connection, id))
            },
            Ok,
        )
}

pub(super) fn map_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get("id")?,
        project: row.get("project")?,
        directory: row.get("directory")?,
        started_at: row.get("started_at")?,
        ended_at: row.get("ended_at")?,
        summary: row.get("summary")?,
    })
}

pub(super) fn get_observation_row(
    connection: &Connection,
    id: i64,
) -> Result<Observation, StoreError> {
    connection
        .query_row(
            &format!("SELECT {OBSERVATION_COLUMNS} FROM observations WHERE id = ?1"),
            [id],
            map_observation,
        )
        .optional()?
        .ok_or(StoreError::ObservationNotFound(id))
}

pub(super) fn map_observation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Observation> {
    Ok(Observation {
        id: row.get("id")?,
        sync_id: row.get("sync_id")?,
        session_id: row.get("session_id")?,
        kind: row.get("type")?,
        title: row.get("title")?,
        content: row.get("content")?,
        tool_name: row.get("tool_name")?,
        project: row.get("project")?,
        scope: row.get("scope")?,
        topic_key: row.get("topic_key")?,
        revision_count: row.get("revision_count")?,
        duplicate_count: row.get("duplicate_count")?,
        last_seen_at: row.get("last_seen_at")?,
        review_after: row.get("review_after")?,
        prompt_sync_id: row.get("prompt_sync_id")?,
        pinned: row.get("pinned")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: row.get("deleted_at")?,
    })
}

pub(super) fn map_relation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Relation> {
    Ok(Relation {
        id: row.get("id")?,
        sync_id: row.get("sync_id")?,
        source_id: row.get("source_id")?,
        target_id: row.get("target_id")?,
        relation: row.get("relation")?,
        reason: row.get("reason")?,
        evidence: row.get("evidence")?,
        confidence: row.get("confidence")?,
        judgment_status: row.get("judgment_status")?,
        marked_by_actor: row.get("marked_by_actor")?,
        marked_by_kind: row.get("marked_by_kind")?,
        marked_by_model: row.get("marked_by_model")?,
        session_id: row.get("session_id")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub(super) fn map_relation_list_item(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RelationListItem> {
    Ok(RelationListItem {
        id: row.get(0)?,
        sync_id: row.get(1)?,
        relation: row.get(2)?,
        judgment_status: row.get(3)?,
        source_id: row.get(4)?,
        source_title: row.get(5)?,
        target_id: row.get(6)?,
        target_title: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

/// One end of a pending pair, starting at column `first`, or nothing when that
/// memory is gone.
///
/// The relation listing reaches both ends through `LEFT JOIN ... deleted_at IS
/// NULL`, so a memory that has been deleted since the pair was proposed arrives
/// as a row of nulls rather than as no row at all. Keyed on the id because that
/// is the column the join can never leave null for a memory that is there.
pub(super) fn pending_side(
    row: &rusqlite::Row<'_>,
    first: usize,
) -> rusqlite::Result<Option<PendingSide>> {
    let Some(id) = row.get::<_, Option<i64>>(first)? else {
        return Ok(None);
    };
    Ok(Some(PendingSide {
        id,
        kind: row.get(first + 1)?,
        title: row.get(first + 2)?,
        topic_key: row.get(first + 3)?,
    }))
}

pub(super) fn collect_prompts_tx(
    tx: &Transaction<'_>,
    project: &str,
) -> Result<Vec<Prompt>, StoreError> {
    let ids = query_column(
        tx,
        "SELECT id FROM prompts WHERE ifnull(project, '') = ?1",
        project,
    )?;
    ids.into_iter().map(|id| get_prompt_tx(tx, id)).collect()
}

pub(super) fn collect_prompts_for_session_tx(
    tx: &Transaction<'_>,
    session_id: &str,
) -> Result<Vec<Prompt>, StoreError> {
    let ids = query_column(
        tx,
        "SELECT id FROM prompts WHERE session_id = ?1",
        session_id,
    )?;
    ids.into_iter().map(|id| get_prompt_tx(tx, id)).collect()
}

pub(super) fn map_deferred_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeferredRow> {
    let payload = row.get::<_, String>(2)?;
    Ok(DeferredRow {
        sync_id: row.get(0)?,
        entity: row.get(1)?,
        payload_valid: serde_json::from_str::<serde_json::Value>(&payload).is_ok(),
        payload,
        apply_status: row.get(3)?,
        retry_count: row.get(4)?,
        last_error: row.get(5)?,
        last_attempted_at: row.get(6)?,
        first_seen_at: row.get(7)?,
    })
}

pub(super) fn get_prompt_tx(tx: &Transaction<'_>, id: i64) -> Result<Prompt, StoreError> {
    tx.query_row(
        &format!("SELECT {PROMPT_COLUMNS} FROM prompts WHERE id = ?1"),
        [id],
        map_prompt,
    )
    .optional()?
    .ok_or_else(|| StoreError::PromptNotFound(id))
}

pub(super) fn map_session_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    Ok(SessionSummary {
        id: row.get(0)?,
        project: row.get(1)?,
        started_at: row.get(2)?,
        ended_at: row.get(3)?,
        summary: row.get(4)?,
        observation_count: row.get(5)?,
        last_activity: row.get(6)?,
    })
}

pub(super) fn map_prompt(row: &rusqlite::Row<'_>) -> rusqlite::Result<Prompt> {
    Ok(Prompt {
        id: row.get("id")?,
        sync_id: row.get("sync_id")?,
        session_id: row.get("session_id")?,
        content: row.get("content")?,
        project: row.get("project")?,
        created_at: row.get("created_at")?,
    })
}

pub(super) fn map_sync_mutation(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncMutation> {
    Ok(SyncMutation {
        seq: row.get("seq")?,
        target_key: row.get("target_key")?,
        entity: row.get("entity")?,
        entity_key: row.get("entity_key")?,
        op: row.get("op")?,
        payload: row.get("payload")?,
        source: row.get("source")?,
        project: row.get("project")?,
        occurred_at: row.get("occurred_at")?,
        acked_at: row.get("acked_at")?,
    })
}

pub(super) fn map_sync_state(row: &rusqlite::Row<'_>) -> rusqlite::Result<SyncState> {
    Ok(SyncState {
        target_key: row.get("target_key")?,
        lifecycle: row.get("lifecycle")?,
        last_enqueued_seq: row.get("last_enqueued_seq")?,
        last_acked_seq: row.get("last_acked_seq")?,
        last_pulled_seq: row.get("last_pulled_seq")?,
        consecutive_failures: row.get("consecutive_failures")?,
        backoff_until: row.get("backoff_until")?,
        lease_owner: row.get("lease_owner")?,
        lease_until: row.get("lease_until")?,
        reason_code: row.get("reason_code")?,
        reason_message: row.get("reason_message")?,
        last_error: row.get("last_error")?,
        updated_at: row.get("updated_at")?,
    })
}

pub(super) fn map_timeline_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<TimelineEntry> {
    Ok(TimelineEntry {
        id: row.get("id")?,
        session_id: row.get("session_id")?,
        kind: row.get("type")?,
        title: row.get("title")?,
        content: row.get("content")?,
        tool_name: row.get("tool_name")?,
        project: row.get("project")?,
        scope: row.get("scope")?,
        topic_key: row.get("topic_key")?,
        revision_count: row.get("revision_count")?,
        duplicate_count: row.get("duplicate_count")?,
        last_seen_at: row.get("last_seen_at")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: row.get("deleted_at")?,
        is_focus: false,
    })
}
