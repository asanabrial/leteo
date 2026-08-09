//! Applying what a peer sent, and journalling what we send back.

use super::*;

pub(super) fn require_sync_target(target_key: &str) -> Result<String, StoreError> {
    let target_key = target_key.trim();
    if target_key.is_empty() {
        Err(invalid_parameter("sync target key is required"))
    } else {
        Ok(target_key.to_owned())
    }
}

fn decode_sync_payload<T: DeserializeOwned>(payload: &str) -> Result<T, StoreError> {
    let payload = payload.trim();
    if payload.starts_with('"') {
        let decoded: String = serde_json::from_str(payload)?;
        Ok(serde_json::from_str(&decoded)?)
    } else {
        Ok(serde_json::from_str(payload)?)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyncSessionPayload {
    pub(super) id: String,
    pub(super) project: String,
    pub(super) directory: String,
    pub(super) started_at: String,
    pub(super) ended_at: Option<String>,
    pub(super) summary: Option<String>,
    pub(super) deleted: bool,
    pub(super) deleted_at: Option<String>,
    pub(super) hard_delete: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct SyncObservationPayload {
    pub(super) sync_id: String,
    pub(super) session_id: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) title: String,
    pub(super) content: String,
    pub(super) tool_name: Option<String>,
    pub(super) project: Option<String>,
    pub(super) scope: String,
    pub(super) topic_key: Option<String>,
    pub(super) prompt_sync_id: Option<String>,
    pub(super) revision_count: i64,
    pub(super) duplicate_count: i64,
    pub(super) last_seen_at: Option<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) deleted: bool,
    pub(super) deleted_at: Option<String>,
    pub(super) hard_delete: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyncPromptPayload {
    pub(super) sync_id: String,
    pub(super) session_id: String,
    pub(super) content: String,
    pub(super) project: Option<String>,
    pub(super) created_at: String,
    pub(super) deleted: bool,
    pub(super) deleted_at: Option<String>,
    pub(super) hard_delete: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SyncRelationPayload {
    pub(super) sync_id: String,
    pub(super) source_id: String,
    pub(super) target_id: String,
    pub(super) relation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) evidence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) confidence: Option<f64>,
    pub(super) judgment_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) marked_by_actor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) marked_by_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) marked_by_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) session_id: Option<String>,
    pub(super) project: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

pub(super) fn apply_sync_mutation_tx(
    tx: &Transaction<'_>,
    mutation: &SyncMutation,
    max_content_bytes: usize,
) -> Result<(), StoreError> {
    let entity = mutation.entity.trim();
    let operation = mutation.op.trim();
    match entity {
        "session" => {
            let mut payload: SyncSessionPayload = decode_sync_payload(&mutation.payload)?;
            if payload.id.trim().is_empty() {
                payload.id = mutation.entity_key.trim().to_owned();
            }
            if operation == crate::sync::OP_DELETE
                || sync_payload_is_deleted(
                    payload.deleted,
                    payload.hard_delete,
                    payload.deleted_at.as_deref(),
                )
            {
                return apply_session_delete_tx(tx, &payload.id);
            }
            apply_session_upsert_tx(tx, payload, max_content_bytes)
        }
        "observation" => {
            let mut payload: SyncObservationPayload = decode_sync_payload(&mutation.payload)?;
            if payload.sync_id.trim().is_empty() {
                payload.sync_id = mutation.entity_key.trim().to_owned();
            }
            if operation == crate::sync::OP_DELETE
                || sync_payload_is_deleted(
                    payload.deleted,
                    payload.hard_delete,
                    payload.deleted_at.as_deref(),
                )
            {
                return apply_observation_delete_tx(tx, payload);
            }
            apply_observation_upsert_tx(tx, payload, max_content_bytes)
        }
        "prompt" => {
            let mut payload: SyncPromptPayload = decode_sync_payload(&mutation.payload)?;
            if payload.sync_id.trim().is_empty() {
                payload.sync_id = mutation.entity_key.trim().to_owned();
            }
            if operation == crate::sync::OP_DELETE
                || sync_payload_is_deleted(
                    payload.deleted,
                    payload.hard_delete,
                    payload.deleted_at.as_deref(),
                )
            {
                return apply_prompt_delete_tx(tx, payload);
            }
            apply_prompt_upsert_tx(tx, payload, max_content_bytes)
        }
        "relation" if operation == crate::sync::OP_UPSERT => {
            apply_relation_upsert_tx(tx, mutation, max_content_bytes)
        }
        _ => Err(invalid_parameter(format!(
            "unsupported sync mutation {entity:?}/{operation:?}"
        ))),
    }
}

fn apply_session_upsert_tx(
    tx: &Transaction<'_>,
    payload: SyncSessionPayload,
    max_content_bytes: usize,
) -> Result<(), StoreError> {
    if payload.id.trim().is_empty() {
        return Err(invalid_parameter("session sync payload id is required"));
    }
    // The project, spelled the way this store spells it. `create_session`
    // normalises and this did not, so a session sent as `Leteo` was stored as
    // `Leteo` — and every query that narrows by project compares against the
    // normalised name, so that session never appeared in an opening context,
    // and the memories hanging off it were attributed to a project nothing
    // else in the store agreed existed.
    //
    // The third of these found today. The observation path had it, was fixed,
    // and neither of its two siblings was looked at.
    let project = normalize::project(&payload.project);
    // And the summary, which nothing normalised on either path: see
    // `normalize::session_summary`.
    let summary = normalize::session_summary(payload.summary.as_deref(), max_content_bytes);
    tx.execute(
        "INSERT INTO sessions (id, project, directory, started_at, ended_at, summary)
         VALUES (?1, ?2, ?3, COALESCE(NULLIF(?4, ''), datetime('now')), ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
             project = excluded.project,
             directory = excluded.directory,
             started_at = COALESCE(NULLIF(excluded.started_at, ''), sessions.started_at),
             ended_at = COALESCE(excluded.ended_at, sessions.ended_at),
             summary = COALESCE(excluded.summary, sessions.summary)",
        params![
            payload.id,
            project,
            payload.directory,
            payload.started_at,
            payload.ended_at,
            summary
        ],
    )?;
    Ok(())
}

fn apply_session_delete_tx(tx: &Transaction<'_>, session_id: &str) -> Result<(), StoreError> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(());
    }
    tx.execute("DELETE FROM prompts WHERE session_id = ?1", [session_id])?;
    tx.execute("DELETE FROM sessions WHERE id = ?1", [session_id])?;
    Ok(())
}

fn apply_observation_upsert_tx(
    tx: &Transaction<'_>,
    payload: SyncObservationPayload,
    max_content_bytes: usize,
) -> Result<(), StoreError> {
    if payload.sync_id.trim().is_empty() || payload.session_id.trim().is_empty() {
        return Err(invalid_parameter(
            "observation sync payload requires sync_id and session_id",
        ));
    }
    let existing = tx
        .query_row(
            "SELECT id, revision_count, duplicate_count, last_seen_at, created_at, updated_at
             FROM observations WHERE sync_id = ?1 ORDER BY id DESC LIMIT 1",
            [&payload.sync_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    let revision_count = if payload.revision_count > 0 {
        payload.revision_count
    } else {
        existing.as_ref().map_or(1, |row| row.1.max(1))
    };
    let duplicate_count = if payload.duplicate_count > 0 {
        payload.duplicate_count
    } else {
        existing.as_ref().map_or(1, |row| row.2.max(1))
    };
    let last_seen_at = payload
        .last_seen_at
        .or_else(|| existing.as_ref().and_then(|row| row.3.clone()));
    let created_at = if payload.created_at.trim().is_empty() {
        existing
            .as_ref()
            .map_or_else(sqlite_now, |row| row.4.clone())
    } else {
        payload.created_at
    };
    let updated_at = if payload.updated_at.trim().is_empty() {
        existing
            .as_ref()
            .map_or_else(|| created_at.clone(), |row| row.5.clone())
    } else {
        payload.updated_at
    };
    // The same rules a local save goes through. This path used to normalise on
    // its own and had fallen behind on five of them, so a memory arriving over
    // the wire kept its private tags, ignored the length cap, and could put back
    // the very type the canonical-types migration had folded away.
    let (kind, title, content, project, scope, topic_key, normalized_hash) = normalize::fields(
        &payload.kind,
        &payload.title,
        &payload.content,
        payload.project.as_deref(),
        &payload.scope,
        payload.topic_key.as_deref(),
        max_content_bytes,
    )
    .into_parts();

    // What the memory used to be, for the review clock below: a peer can change
    // a memory's type, and the three types that are ever due for review are
    // decided by that word.
    let previous_kind: Option<String> = existing
        .as_ref()
        .map(|(id, ..)| {
            tx.query_row("SELECT type FROM observations WHERE id = ?1", [id], |row| {
                row.get(0)
            })
        })
        .transpose()?;
    if let Some((id, ..)) = existing {
        tx.execute(
            "UPDATE observations SET
                 session_id = ?1, type = ?2, title = ?3, content = ?4, tool_name = ?5,
                 project = ?6, scope = ?7, topic_key = ?8, normalized_hash = ?9,
                 revision_count = ?10, duplicate_count = ?11, last_seen_at = ?12,
                 created_at = ?13, updated_at = ?14, deleted_at = NULL,
                 prompt_sync_id = coalesce(?16, prompt_sync_id)
             WHERE id = ?15",
            params![
                payload.session_id,
                kind,
                title,
                content,
                payload.tool_name,
                project,
                scope,
                topic_key,
                normalized_hash,
                revision_count,
                duplicate_count,
                last_seen_at,
                created_at,
                updated_at,
                id,
                payload.prompt_sync_id
            ],
        )?;
    } else {
        tx.execute(
            "INSERT INTO observations
             (sync_id, session_id, type, title, content, tool_name, project, scope, topic_key,
              normalized_hash, revision_count, duplicate_count, last_seen_at,
              created_at, updated_at, deleted_at, prompt_sync_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, ?16)",
            params![
                payload.sync_id,
                payload.session_id,
                kind,
                title,
                content,
                payload.tool_name,
                project,
                scope,
                topic_key,
                normalized_hash,
                revision_count,
                duplicate_count,
                last_seen_at,
                created_at,
                updated_at,
                payload.prompt_sync_id
            ],
        )?;
    }
    // The review clock, which this path never wound. `review_after` is not on
    // the wire at all — no payload carries it — so a decision arriving from a
    // peer had no date and `mem_review` would never name it. The same rule the
    // local save follows, applied to the memory that just landed.
    let id: i64 = tx.query_row(
        "SELECT id FROM observations WHERE sync_id = ?1 ORDER BY id DESC LIMIT 1",
        [&payload.sync_id],
        |row| row.get(0),
    )?;
    crate::store::observations::reschedule_review(tx, id, &kind, previous_kind.as_deref())?;
    Ok(())
}

fn apply_observation_delete_tx(
    tx: &Transaction<'_>,
    payload: SyncObservationPayload,
) -> Result<(), StoreError> {
    if payload.sync_id.trim().is_empty() {
        return Ok(());
    }
    let id = tx
        .query_row(
            "SELECT id FROM observations WHERE sync_id = ?1 ORDER BY id DESC LIMIT 1",
            [&payload.sync_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Ok(());
    };
    if payload.hard_delete {
        orphan_relations_tx(tx, &payload.sync_id)?;
        tx.execute("DELETE FROM observations WHERE id = ?1", [id])?;
    } else {
        let deleted_at = payload
            .deleted_at
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(sqlite_now);
        tx.execute(
            "UPDATE observations SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![deleted_at, id],
        )?;
    }
    Ok(())
}

fn apply_prompt_upsert_tx(
    tx: &Transaction<'_>,
    payload: SyncPromptPayload,
    max_content_bytes: usize,
) -> Result<(), StoreError> {
    if payload.sync_id.trim().is_empty() || payload.session_id.trim().is_empty() {
        return Err(invalid_parameter(
            "prompt sync payload requires sync_id and session_id",
        ));
    }
    let tombstone = tx
        .query_row(
            "SELECT deleted_at FROM prompt_deletions WHERE sync_id = ?1",
            [&payload.sync_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(deleted_at) = tombstone {
        if normalize_comparable_timestamp(&payload.created_at)
            <= normalize_comparable_timestamp(&deleted_at)
        {
            return Ok(());
        }
        tx.execute(
            "DELETE FROM prompt_deletions WHERE sync_id = ?1",
            [&payload.sync_id],
        )?;
    }
    // The same rules a typed prompt goes through. This path had none of them:
    // a prompt arriving over the wire kept its `<private>` spans, ignored the
    // length cap, and stored the project name however it was spelled — so
    // `Leteo` never matched the `leteo` every query narrows by, and that prompt
    // was invisible to the opening context for ever. The observation path was
    // fixed for exactly this and its sibling was left behind.
    let (content, project) = normalize::prompt_fields(
        &payload.content,
        payload.project.as_deref(),
        max_content_bytes,
    );
    let existing = tx
        .query_row(
            "SELECT id FROM prompts WHERE sync_id = ?1 ORDER BY id DESC LIMIT 1",
            [&payload.sync_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(id) = existing {
        tx.execute(
            "UPDATE prompts SET session_id = ?1, content = ?2, project = ?3,
                 created_at = CASE WHEN ?4 = '' THEN created_at ELSE ?4 END
             WHERE id = ?5",
            params![payload.session_id, content, project, payload.created_at, id],
        )?;
    } else if payload.created_at.trim().is_empty() {
        tx.execute(
            "INSERT INTO prompts (sync_id, session_id, content, project)
             VALUES (?1, ?2, ?3, ?4)",
            params![payload.sync_id, payload.session_id, content, project],
        )?;
    } else {
        tx.execute(
            "INSERT INTO prompts (sync_id, session_id, content, project, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                payload.sync_id,
                payload.session_id,
                content,
                project,
                payload.created_at
            ],
        )?;
    }
    Ok(())
}

fn apply_prompt_delete_tx(
    tx: &Transaction<'_>,
    payload: SyncPromptPayload,
) -> Result<(), StoreError> {
    if payload.sync_id.trim().is_empty() {
        return Ok(());
    }
    tx.execute("DELETE FROM prompts WHERE sync_id = ?1", [&payload.sync_id])?;
    let deleted_at = payload
        .deleted_at
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(sqlite_now);
    tx.execute(
        "INSERT INTO prompt_deletions (sync_id, session_id, project, deleted_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(sync_id) DO UPDATE SET
             session_id = excluded.session_id,
             project = excluded.project,
             deleted_at = excluded.deleted_at",
        params![
            payload.sync_id,
            payload.session_id,
            payload.project.unwrap_or_default(),
            deleted_at
        ],
    )?;
    Ok(())
}

pub(super) fn apply_relation_upsert_tx(
    tx: &Transaction<'_>,
    mutation: &SyncMutation,
    max_content_bytes: usize,
) -> Result<(), StoreError> {
    let mut payload: SyncRelationPayload = decode_sync_payload(&mutation.payload)?;
    if payload.sync_id.trim().is_empty() {
        payload.sync_id = mutation.entity_key.trim().to_owned();
    }
    if payload.sync_id.trim().is_empty()
        || payload.source_id.trim().is_empty()
        || payload.target_id.trim().is_empty()
    {
        return Err(invalid_parameter(
            "relation sync payload requires sync_id, source_id, and target_id",
        ));
    }
    let observations = tx.query_row(
        "SELECT COUNT(*) FROM observations WHERE sync_id IN (?1, ?2)",
        params![payload.source_id, payload.target_id],
        |row| row.get::<_, i64>(0),
    )?;
    if observations < 2 {
        tx.execute(
            "INSERT INTO sync_deferred_mutations
             (sync_id, entity, payload, apply_status, retry_count, first_seen_at, last_attempted_at)
             VALUES (?1, 'relation', ?2, 'deferred', 0, datetime('now'), datetime('now'))
             ON CONFLICT(sync_id) DO UPDATE SET
                 payload = excluded.payload,
                 apply_status = 'deferred',
                 last_attempted_at = datetime('now')",
            params![payload.sync_id, mutation.payload],
        )?;
        return Ok(());
    }
    // A confidence the local path would have refused is stored as no
    // confidence at all. `judge_relation` checks the 0..=1 range before it
    // writes and this never did, so a peer could put any number in a column
    // every reader treats as a probability.
    //
    // Dropped rather than clamped, and rather than refused. Clamping invents a
    // number nobody produced; refusing loses a peer's judgment, which is the
    // reason normalisation lives on this path and rejection stays at the door.
    // `NULL` is what the store already means by "nobody said".
    //
    // The verb is deliberately left as it arrived, and that is not an
    // oversight. An unrecognised one is inert — `caveats_for` reads two verbs
    // by name and `relation_stats` counts what it finds — while folding it to
    // `pending` would turn a peer's judged relation back into a question, this
    // store would send that back, and two machines that disagree about a verb
    // would hand it to each other for ever.
    let confidence = payload
        .confidence
        .filter(|value| crate::memory::rules::is_confidence(*value));
    let created_at = nonempty_or_now(&payload.created_at);
    let updated_at = if payload.updated_at.trim().is_empty() {
        created_at.clone()
    } else {
        payload.updated_at
    };
    // The same rules a locally judged relation goes through: see
    // `normalize::judgment_text`. Neither path had them.
    let reason = normalize::judgment_text(payload.reason.as_deref(), max_content_bytes);
    let evidence = normalize::judgment_text(payload.evidence.as_deref(), max_content_bytes);
    tx.execute(
        &format!(
            "INSERT INTO memory_relations ({RELATION_INSERT_COLUMNS})
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
         ON CONFLICT(sync_id) DO UPDATE SET
             source_id = excluded.source_id,
             target_id = excluded.target_id,
             relation = excluded.relation,
             reason = excluded.reason,
             evidence = excluded.evidence,
             confidence = excluded.confidence,
             judgment_status = excluded.judgment_status,
             marked_by_actor = excluded.marked_by_actor,
             marked_by_kind = excluded.marked_by_kind,
             marked_by_model = excluded.marked_by_model,
             session_id = excluded.session_id,
             updated_at = excluded.updated_at"
        ),
        params![
            payload.sync_id,
            payload.source_id,
            payload.target_id,
            payload.relation,
            reason,
            evidence,
            confidence,
            payload.judgment_status,
            payload.marked_by_actor,
            payload.marked_by_kind,
            payload.marked_by_model,
            payload.session_id,
            created_at,
            updated_at
        ],
    )?;
    tx.execute(
        "DELETE FROM sync_deferred_mutations WHERE sync_id = ?1",
        [&payload.sync_id],
    )?;
    Ok(())
}

/// Drops journal rows a target already acknowledged and has had time to
/// reconcile.
///
/// Each row carries a full JSON copy of the observation, session, or prompt it
/// describes, so an unpruned journal roughly doubles the size of the database.
/// An acknowledged row is never sent again — `list_pending_sync_mutations`
/// only reads rows with no `acked_at` — and `seq` is `AUTOINCREMENT`, so the
/// identifiers of deleted rows are never reused and no cursor can go backwards.
/// The retention window leaves a few days of history for anyone debugging a
/// sync problem.
pub(super) fn prune_acked_mutations_tx(
    tx: &Transaction<'_>,
    target_key: &str,
) -> Result<usize, StoreError> {
    let cutoff = format!("-{ACKED_MUTATION_RETENTION_DAYS} days");
    Ok(tx.execute(
        "DELETE FROM sync_mutations
         WHERE target_key = ?1 AND acked_at IS NOT NULL
           AND datetime(acked_at) < datetime('now', ?2)",
        params![target_key, cutoff],
    )?)
}

pub(super) fn record_deferred_attempt_tx(
    tx: &Transaction<'_>,
    sync_id: &str,
    retry_count: i64,
    dead: bool,
    last_error: &str,
) -> Result<(), StoreError> {
    let status = if dead { "dead" } else { "deferred" };
    tx.execute(
        "UPDATE sync_deferred_mutations
         SET retry_count = ?2, apply_status = ?3, last_error = ?4,
             last_attempted_at = datetime('now')
         WHERE sync_id = ?1",
        params![sync_id, retry_count, status, last_error],
    )?;
    Ok(())
}

pub(super) fn replay_deferred_relations_tx(
    tx: &Transaction<'_>,
    max_content_bytes: usize,
) -> Result<(), StoreError> {
    let pending = {
        let mut statement = tx.prepare(
            "SELECT sync_id, payload FROM sync_deferred_mutations
             WHERE entity = 'relation' AND apply_status = 'deferred'
             ORDER BY first_seen_at, sync_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (sync_id, payload) in pending {
        apply_relation_upsert_tx(
            tx,
            &SyncMutation {
                entity: "relation".to_owned(),
                entity_key: sync_id,
                op: crate::sync::OP_UPSERT.to_owned(),
                payload,
                source: "remote".to_owned(),
                ..SyncMutation::default()
            },
            max_content_bytes,
        )?;
    }
    Ok(())
}

pub(super) fn enqueue_relation_if_enrolled(
    tx: &Transaction<'_>,
    relation: &Relation,
    source_project: &str,
    target_project: &str,
) -> Result<(), StoreError> {
    let enrollment_project = if source_project.is_empty() {
        target_project
    } else {
        source_project
    };
    let enrolled = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sync_enrolled_projects WHERE project = ?1
         )",
        [enrollment_project],
        |row| row.get::<_, bool>(0),
    )?;
    if !enrolled {
        return Ok(());
    }
    let payload = serde_json::json!({
        "sync_id": relation.sync_id,
        "source_id": relation.source_id,
        "target_id": relation.target_id,
        "relation": relation.relation,
        "reason": relation.reason,
        "evidence": relation.evidence,
        "confidence": relation.confidence,
        "judgment_status": relation.judgment_status,
        "marked_by_actor": relation.marked_by_actor,
        "marked_by_kind": relation.marked_by_kind,
        "marked_by_model": relation.marked_by_model,
        "session_id": relation.session_id,
        "project": source_project,
        "created_at": relation.created_at,
        "updated_at": relation.updated_at,
    });
    let payload = serde_json::to_string(&payload)?;
    enqueue_serialized_mutation(
        tx,
        "relation",
        &relation.sync_id,
        crate::sync::OP_UPSERT,
        &payload,
        source_project,
    )
}

pub(super) fn enqueue_prompt_delete_tx(
    tx: &Transaction<'_>,
    prompt: &Prompt,
) -> Result<(), StoreError> {
    enqueue_mutation(
        tx,
        "prompt",
        &prompt.sync_id,
        crate::sync::OP_DELETE,
        &serde_json::json!({
            "sync_id": prompt.sync_id,
            "session_id": prompt.session_id,
            "project": prompt.project,
            "deleted": true,
            "deleted_at": sqlite_now(),
            "hard_delete": true,
        }),
        &prompt.project,
    )
}

pub(super) fn enqueue_session_delete_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    project: &str,
) -> Result<(), StoreError> {
    enqueue_mutation(
        tx,
        "session",
        session_id,
        crate::sync::OP_DELETE,
        &serde_json::json!({
            "id": session_id,
            "project": project,
            "deleted": true,
            "deleted_at": sqlite_now(),
        }),
        project,
    )
}

pub(super) fn enqueue_observation(
    tx: &Transaction<'_>,
    observation: &Observation,
) -> Result<(), StoreError> {
    // Pinning does not travel.
    //
    // It says where *this* store looks first, not what the memory is, and one
    // machine's shelf should not rearrange everybody else's. Engram settled the
    // same way and wrote a test to hold it — Leteo inherited the behaviour
    // through a `#[serde(skip)]` on the field, with nothing saying why, and
    // that same attribute was also dropping the pin from `leteo export`, which
    // is a different question with a different answer.
    //
    // So the field serialises now, and the rule lives here instead: the one
    // place that builds what a peer receives. The clone is a body's worth of
    // allocation on a path that is about to allocate the whole payload as JSON
    // anyway.
    let mut travelling = observation.clone();
    travelling.pinned = false;
    enqueue_mutation(
        tx,
        "observation",
        &observation.sync_id,
        crate::sync::OP_UPSERT,
        &travelling,
        observation.project.as_deref().unwrap_or_default(),
    )
}

/// Queues everything a project already holds, as though it had just been saved.
///
/// The same payloads the live journal writes, built the same way, so the cloud
/// cannot tell a catch-up from an ordinary write.
pub(super) fn backfill_project_tx(tx: &Transaction<'_>, project: &str) -> Result<(), StoreError> {
    // Sessions first, and the order is the whole correctness of this.
    //
    // Mutations are numbered as they are queued and a peer applies them in that
    // order, so what is queued first is what exists first at the other end. Both
    // `observations` and `prompts` carry `FOREIGN KEY (session_id) REFERENCES
    // sessions(id)`, and the backfill used to queue every observation before
    // any session — so the first memory a peer tried to apply failed with
    // `FOREIGN KEY constraint failed`, and so did every one after it.
    //
    // That error comes back from `apply_pulled_sync_mutation` as a store error,
    // which fails the pull, which records a failure and backs off. The next
    // attempt fetches the same page and fails identically. The sync never
    // recovers and the diagnosis on offer is five words from SQLite.
    //
    // It is not an edge case: enrolling a project that already holds memories
    // is the ordinary first run of cloud sync, and it is the only path that
    // queues a session and its observations in one go. Live writes were always
    // safe — a session is created before anything can reference it, so its
    // mutation already has the lower sequence.
    for id in query_column::<String>(tx, "SELECT id FROM sessions WHERE project = ?1", project)? {
        let session = get_session_row(tx, &id)?;
        enqueue_mutation(
            tx,
            "session",
            &id,
            crate::sync::OP_UPSERT,
            &session,
            project,
        )?;
    }
    let observations = query_column(
        tx,
        "SELECT id FROM observations
         WHERE LOWER(ifnull(project, '')) = ?1 AND deleted_at IS NULL",
        project,
    )?;
    for id in observations {
        let observation = get_observation_row(tx, id)?;
        enqueue_observation(tx, &observation)?;
    }
    let prompts = query_column(
        tx,
        "SELECT id FROM prompts WHERE LOWER(ifnull(project, '')) = ?1",
        project,
    )?;
    for id in prompts {
        let prompt = get_prompt_tx(tx, id)?;
        enqueue_mutation(
            tx,
            "prompt",
            &prompt.sync_id,
            crate::sync::OP_UPSERT,
            &prompt,
            project,
        )?;
    }
    backfill_relations_tx(tx, project)
}

/// The judged graph, which the backfill used to leave behind.
///
/// Sessions, observations and prompts — and not a single relation. So a project
/// enrolled after any of it had been curated arrived at the peer as bare
/// memories: every *this supersedes that* and every *not a conflict* verdict
/// stayed on the machine that made them. On the other side the superseded
/// decisions read as current, and the same pairs come back up for judgement to
/// be answered a second time.
///
/// It is the same defect an export had, and was fixed there: "an export was
/// sessions, observations and prompts, so a store exported and imported back
/// came home without a single relation". Two pipes, one omission, found by
/// enrolling a project that already held a verdict and counting what queued.
///
/// Last, and that is the order it has to be in: a relation names two memories
/// by `sync_id`, and a peer applies mutations in the order they were queued.
///
/// Only what points at two live memories of this project, which is the same
/// rule `export_relation_mutations` follows — a relation to a deleted memory is
/// not something to teach a peer.
fn backfill_relations_tx(tx: &Transaction<'_>, project: &str) -> Result<(), StoreError> {
    let mut statement = tx.prepare(
        "SELECT r.sync_id, ifnull(r.source_id, ''), ifnull(r.target_id, ''), r.relation,
                r.reason, r.evidence, r.confidence, r.judgment_status,
                r.marked_by_actor, r.marked_by_kind, r.marked_by_model, r.session_id,
                r.created_at, r.updated_at
           FROM memory_relations r
           JOIN observations src ON src.sync_id = r.source_id AND src.deleted_at IS NULL
           JOIN observations target ON target.sync_id = r.target_id AND target.deleted_at IS NULL
           LEFT JOIN sessions src_session ON src_session.id = src.session_id
           LEFT JOIN sessions target_session ON target_session.id = target.session_id
          WHERE r.judgment_status != 'orphaned'
            AND LOWER(coalesce(nullif(src.project, ''), src_session.project, '')) = ?1
            AND LOWER(coalesce(nullif(target.project, ''), target_session.project, '')) = ?1
          ORDER BY datetime(r.created_at), r.sync_id",
    )?;
    let payloads = statement
        .query_map([project], |row| {
            let sync_id: String = row.get(0)?;
            Ok((
                sync_id.clone(),
                serde_json::json!({
                    "sync_id": sync_id,
                    "source_id": row.get::<_, String>(1)?,
                    "target_id": row.get::<_, String>(2)?,
                    "relation": row.get::<_, String>(3)?,
                    "reason": row.get::<_, Option<String>>(4)?,
                    "evidence": row.get::<_, Option<String>>(5)?,
                    "confidence": row.get::<_, Option<f64>>(6)?,
                    "judgment_status": row.get::<_, String>(7)?,
                    "marked_by_actor": row.get::<_, Option<String>>(8)?,
                    "marked_by_kind": row.get::<_, Option<String>>(9)?,
                    "marked_by_model": row.get::<_, Option<String>>(10)?,
                    "session_id": row.get::<_, Option<String>>(11)?,
                    "project": project,
                    "created_at": row.get::<_, String>(12)?,
                    "updated_at": row.get::<_, String>(13)?,
                }),
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (sync_id, payload) in payloads {
        enqueue_serialized_mutation(
            tx,
            "relation",
            &sync_id,
            crate::sync::OP_UPSERT,
            &serde_json::to_string(&payload)?,
            project,
        )?;
    }
    Ok(())
}

pub(super) fn is_enrolled_tx(tx: &Transaction<'_>, project: &str) -> Result<bool, StoreError> {
    if project.is_empty() {
        return Ok(true);
    }
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_enrolled_projects WHERE project = ?1)",
        [project],
        |row| row.get(0),
    )?)
}

pub(super) fn enqueue_mutation<T: Serialize>(
    tx: &Transaction<'_>,
    entity: &str,
    entity_key: &str,
    operation: &str,
    payload: &T,
    project: &str,
) -> Result<(), StoreError> {
    let payload = serde_json::to_string(payload)?;
    let mut project = normalize::project(project);
    if project.is_empty() {
        let value: serde_json::Value = serde_json::from_str(&payload)?;
        project = value
            .get("project")
            .and_then(serde_json::Value::as_str)
            .map(normalize::project)
            .unwrap_or_default();
        if project.is_empty()
            && let Some(session_id) = value
                .get("session_id")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
        {
            project = tx
                .query_row(
                    "SELECT project FROM sessions WHERE id = ?1",
                    [session_id],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or_default();
        }
    }
    enqueue_serialized_mutation(tx, entity, entity_key, operation, &payload, &project)
}

fn enqueue_serialized_mutation(
    tx: &Transaction<'_>,
    entity: &str,
    entity_key: &str,
    operation: &str,
    payload: &str,
    project: &str,
) -> Result<(), StoreError> {
    // Nothing is journalled for a project nobody replicates. Enrolling one
    // queues what it already holds, so this loses no history.
    if !is_enrolled_tx(tx, project)? {
        return Ok(());
    }
    tx.execute(
        "INSERT OR IGNORE INTO sync_state (target_key, lifecycle, updated_at) VALUES ('cloud', 'idle', datetime('now'))",
        [],
    )?;
    tx.execute(
        "INSERT INTO sync_mutations (target_key, entity, entity_key, op, payload, source, project)
         VALUES ('cloud', ?1, ?2, ?3, ?4, 'local', ?5)",
        params![entity, entity_key, operation, payload, project],
    )?;
    let sequence = tx.last_insert_rowid();
    tx.execute(
        "UPDATE sync_state SET lifecycle = 'pending', last_enqueued_seq = ?1, updated_at = datetime('now') WHERE target_key = 'cloud'",
        [sequence],
    )?;
    if !project.is_empty() {
        let project_target = format!("cloud:{project}");
        tx.execute(ENSURE_SYNC_TARGET, [&project_target])?;
        tx.execute(
            "UPDATE sync_state SET lifecycle = 'pending', last_enqueued_seq = ?1, updated_at = datetime('now') WHERE target_key = ?2",
            params![sequence, project_target],
        )?;
    }
    Ok(())
}
