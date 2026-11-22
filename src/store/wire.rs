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
    pub(super) directory: Option<String>,
    pub(super) revision_count: i64,
    pub(super) duplicate_count: i64,
    pub(super) last_seen_at: Option<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) deleted: bool,
    pub(super) deleted_at: Option<String>,
    pub(super) hard_delete: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SyncObservationVersionPayload {
    /// The observation this version belongs to, by the identifier that survives
    /// replication. Named `sync_id` rather than `observation_sync_id` so it
    /// matches the key every other entity's payload carries.
    pub(super) sync_id: String,
    /// The `revision_count` the live row held before the write that replaced it.
    pub(super) revision: i64,
    pub(super) title: String,
    pub(super) content: String,
    /// The moment of replacement. Carried so both machines store the same
    /// string: a value each side generated would disagree.
    pub(super) replaced_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) project: Option<String>,
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
        "observation_version" => {
            let mut payload: SyncObservationVersionPayload =
                decode_sync_payload(&mutation.payload)?;
            if payload.sync_id.trim().is_empty() {
                payload.sync_id = mutation.entity_key.trim().to_owned();
            }
            // A version is only ever written, never removed: the row a later
            // write replaced is the row this one keeps, and there is nothing a
            // delete could name.
            if operation != crate::sync::OP_UPSERT {
                return Ok(());
            }
            apply_observation_version_upsert_tx(tx, payload, max_content_bytes)
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
        crate::sync::ENTITY_PROJECT_RENAME if operation == crate::sync::OP_UPSERT => {
            let mut payload: crate::sync::ProjectRenameMutationPayload =
                decode_sync_payload(&mutation.payload)?;
            if payload.sync_id.trim().is_empty() {
                payload.sync_id = mutation.entity_key.trim().to_owned();
            }
            if payload.project.trim().is_empty() {
                payload.project.clone_from(&mutation.project);
            }
            apply_project_rename_tx(tx, payload)
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
            crate::project::recordable_directory(&payload.directory),
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
    // Through the rule a local write follows, so a peer cannot hand over a
    // relative path the local door would refuse. Absent keeps what this store
    // holds: an older peer does not send the field, and its silence is not a
    // statement that the memory has no origin.
    let directory = normalize::directory(payload.directory.as_deref());

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
                 prompt_sync_id = coalesce(?16, prompt_sync_id),
                 directory = coalesce(?17, directory)
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
                payload.prompt_sync_id,
                directory
            ],
        )?;
    } else {
        tx.execute(
            "INSERT INTO observations
             (sync_id, session_id, type, title, content, tool_name, project, scope, topic_key,
              normalized_hash, revision_count, duplicate_count, last_seen_at,
              created_at, updated_at, deleted_at, prompt_sync_id, directory)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, NULL, ?16, ?17)",
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
                payload.prompt_sync_id,
                directory
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

/// Records the version a peer sent.
///
/// The title and body go through the same two normalisers a local write used on
/// its way in — redaction and the length bound — because a payload is a door
/// like any other, and the `[REDACTED]` promise covers the journal a replica
/// replays: without this a peer could store a `<private>` span the local path
/// can never write and hand it back through `include_history`. Both normalisers
/// are idempotent, so the bytes an honest origin already redacted and bounded
/// come back unchanged and the two stores still hold the same version — which
/// is why this is not the "second opinion" the note here once feared. The unique
/// index on `(observation_sync_id, revision)` plus `INSERT OR IGNORE` is what
/// makes a repeated payload a no-op.
fn apply_observation_version_upsert_tx(
    tx: &Transaction<'_>,
    payload: SyncObservationVersionPayload,
    max_content_bytes: usize,
) -> Result<(), StoreError> {
    if payload.sync_id.trim().is_empty() || payload.revision <= 0 {
        return Err(invalid_parameter(
            "observation version payload requires sync_id and a positive revision",
        ));
    }
    // The version of a memory a hard delete already removed. The journal queues
    // a version before its memory's own upsert, so "no memory here" cannot be
    // the test: only a recorded delete says the text is unwanted.
    let deleted: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM observation_deletions WHERE sync_id = ?1)",
        [payload.sync_id.trim()],
        |row| row.get(0),
    )?;
    if deleted {
        return Ok(());
    }
    let replaced_at = if payload.replaced_at.trim().is_empty() {
        sqlite_now()
    } else {
        payload.replaced_at
    };
    let title = normalize::title(&payload.title, max_content_bytes);
    let content = normalize::stored_content(&payload.content, max_content_bytes).0;
    crate::store::observations::snapshot_observation_version_tx(
        tx,
        payload.sync_id.trim(),
        payload.revision,
        &title,
        &content,
        &replaced_at,
    )?;
    Ok(())
}

fn apply_observation_delete_tx(
    tx: &Transaction<'_>,
    payload: SyncObservationPayload,
) -> Result<(), StoreError> {
    if payload.sync_id.trim().is_empty() {
        return Ok(());
    }
    let row = tx
        .query_row(
            "SELECT id, project FROM observations WHERE sync_id = ?1 ORDER BY id DESC LIMIT 1",
            [&payload.sync_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    let Some((id, current_project)) = row else {
        // A hard delete for a memory this store never held still has to leave
        // its tombstone: the versions the origin queued before it can arrive
        // after it, and with no row here nothing else would say they are
        // unwanted.
        if payload.hard_delete {
            crate::store::observations::erase_hard_deleted_observation_tx(tx, &payload.sync_id)?;
        }
        return Ok(());
    };
    // A delete says "gone from the project it was filed under". Moving a memory
    // out of an enrolled project queues exactly that, the pull hands this
    // machine its own pushes back, and applying it by `sync_id` alone deleted
    // the memory from the project it had just been moved into.
    let filed_under = normalize::project(payload.project.as_deref().unwrap_or_default());
    if filed_under != normalize::project(current_project.as_deref().unwrap_or_default()) {
        return Ok(());
    }
    if payload.hard_delete {
        orphan_relations_tx(tx, &payload.sync_id)?;
        crate::store::observations::erase_hard_deleted_observation_tx(tx, &payload.sync_id)?;
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

    // One pair holds one relation. Two teams that judge the same pair mint two
    // `sync_id`s, possibly in opposite directions, and keying only on
    // `sync_id` inserted the second beside the first: `not_superseded()` then
    // saw both, so a pair could hide one memory from each side. The row already
    // here keeps its `sync_id` and takes the incoming content when that is
    // newer; an older or equally old one is dropped, since the row here is at
    // least as recent a word on the pair.
    let same_sync_id_here = tx
        .query_row(
            "SELECT 1 FROM memory_relations WHERE sync_id = ?1",
            [&payload.sync_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    let other_row_for_pair = if same_sync_id_here {
        None
    } else {
        tx.query_row(
            "SELECT sync_id, datetime(updated_at) < datetime(?4) FROM memory_relations
             WHERE sync_id <> ?1
               AND ((source_id = ?2 AND target_id = ?3) OR (source_id = ?3 AND target_id = ?2))
             ORDER BY id LIMIT 1",
            params![
                payload.sync_id,
                payload.source_id,
                payload.target_id,
                updated_at
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
        )
        .optional()?
    };
    if let Some((existing_sync_id, incoming_is_newer)) = other_row_for_pair {
        if incoming_is_newer {
            tx.execute(
                "UPDATE memory_relations SET
                     source_id = ?2, target_id = ?3, relation = ?4, reason = ?5,
                     evidence = ?6, confidence = ?7, judgment_status = ?8,
                     marked_by_actor = ?9, marked_by_kind = ?10,
                     marked_by_model = ?11, session_id = ?12, updated_at = ?13
                 WHERE sync_id = ?1",
                params![
                    existing_sync_id,
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
                    updated_at
                ],
            )?;
        }
        tx.execute(
            "DELETE FROM sync_deferred_mutations WHERE sync_id = ?1",
            [&payload.sync_id],
        )?;
        return Ok(());
    }
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

/// Every project the journal shows a session was queued under, besides the one
/// it is filed in.
///
/// Where `enqueue_session_for_member_tx` sent it. That function decides by
/// reading the journal, and so does this: a session has no record of the
/// memories that took it to another project beyond the mutation that did.
fn session_member_projects_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    own_project: &str,
) -> Result<Vec<String>, StoreError> {
    let own = normalize::project(own_project);
    let mut statement = tx.prepare(
        "SELECT DISTINCT project FROM sync_mutations
          WHERE entity = 'session' AND entity_key = ?1 AND op = ?2 AND project <> ''
          ORDER BY project",
    )?;
    let projects = statement
        .query_map(params![session_id, crate::sync::OP_UPSERT], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(projects
        .into_iter()
        .filter(|project| *project != own)
        .collect())
}

/// Queues a session's delete under its own project and under every project it
/// was sent to.
///
/// Under its own alone, a peer that follows only the project of a memory filed
/// into this session was never told the session was gone and kept an orphan row
/// for good. The delete does not ask whether memories are still there: the
/// session is going, and so is every copy of it a peer holds.
pub(super) fn enqueue_session_delete_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    project: &str,
) -> Result<(), StoreError> {
    let mut projects = vec![normalize::project(project)];
    projects.extend(session_member_projects_tx(tx, session_id, project)?);
    for project in projects {
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
            &project,
        )?;
    }
    Ok(())
}

/// Queues a changed session under its own project and under each project it
/// was sent to that still holds a memory or prompt of it.
///
/// `end_session` queued under the session's own project, so the peer that
/// follows a memory's project went on holding a session that never ended. The
/// rows are asked about, unlike in the delete: a session whose memories have
/// moved to another project is no longer a member of the one they left, and
/// telling that project's peers about it would put the old project name back on
/// the session they hold.
pub(super) fn enqueue_session_update_tx(
    tx: &Transaction<'_>,
    session: &Session,
) -> Result<(), StoreError> {
    let mut projects = vec![normalize::project(&session.project)];
    for project in session_member_projects_tx(tx, &session.id, &session.project)? {
        let still_a_member: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM observations
                            WHERE session_id = ?1 AND ifnull(project, '') = ?2)
                 OR EXISTS(SELECT 1 FROM prompts
                            WHERE session_id = ?1 AND ifnull(project, '') = ?2)",
            params![session.id, project],
            |row| row.get(0),
        )?;
        if still_a_member {
            projects.push(project);
        }
    }
    for project in projects {
        enqueue_mutation(
            tx,
            "session",
            &session.id,
            crate::sync::OP_UPSERT,
            session,
            &project,
        )?;
    }
    Ok(())
}

/// Queues a session under the project of a row that points at it, when that is
/// not the session's own and the session has not already gone there.
///
/// A session is queued under its own project, and a memory or prompt carries a
/// project of its own that nothing forces to match: `consolidate` run from
/// another directory files the merged memory into a `manual-save-<cwd>` session
/// beside sources of a different project. When only the memory's project is
/// enrolled the session's mutation is dropped, the peer receives a row whose
/// `session_id` it has never heard of, and the foreign key fails on every pull
/// from then on. Export selects such a session for the same reason.
///
/// Once per project and session, which is the same economy `create_session`
/// keeps: a memory per save would otherwise leave a copy of the session per
/// memory in the journal.
pub(super) fn enqueue_session_for_member_tx(
    tx: &Transaction<'_>,
    session_id: &str,
    member_project: &str,
) -> Result<(), StoreError> {
    let project = normalize::project(member_project);
    if project.is_empty() || session_id.trim().is_empty() || !is_enrolled_tx(tx, &project)? {
        return Ok(());
    }
    let session = get_session_row(tx, session_id)?;
    if normalize::project(&session.project) == project {
        return Ok(());
    }
    let already_queued: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_mutations
          WHERE entity = 'session' AND entity_key = ?1 AND project = ?2 AND op = ?3)",
        params![session_id, project, crate::sync::OP_UPSERT],
        |row| row.get(0),
    )?;
    if already_queued {
        return Ok(());
    }
    enqueue_mutation(
        tx,
        "session",
        session_id,
        crate::sync::OP_UPSERT,
        &session,
        &project,
    )
}

pub(super) fn enqueue_observation(
    tx: &Transaction<'_>,
    observation: &Observation,
) -> Result<(), StoreError> {
    enqueue_session_for_member_tx(
        tx,
        &observation.session_id,
        observation.project.as_deref().unwrap_or_default(),
    )?;
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

/// Queues one superseded version, under the observation's own enrolment.
///
/// The project is the one the memory is filed in, resolved by the item that
/// owns the version, so a version is never replicated without the memory it
/// belongs to or into a project nobody enrolled. An empty project means the
/// memory could not be attributed to one, and — exactly as `enqueue_observation`
/// does — the version is queued anyway rather than dropped in silence.
pub(super) fn enqueue_observation_version(
    tx: &Transaction<'_>,
    observation_sync_id: &str,
    revision: i64,
    title: &str,
    content: &str,
    replaced_at: &str,
    project: &str,
) -> Result<(), StoreError> {
    if observation_sync_id.is_empty() {
        return Ok(());
    }
    let payload = SyncObservationVersionPayload {
        sync_id: observation_sync_id.to_owned(),
        revision,
        title: title.to_owned(),
        content: content.to_owned(),
        replaced_at: replaced_at.to_owned(),
        project: None,
    };
    // The entity key is the observation rather than the pair: a version is a
    // property of that memory, and the revision inside the payload is what
    // tells two of them apart on apply. The unique index, not this key, is what
    // makes the apply idempotent.
    enqueue_mutation(
        tx,
        "observation_version",
        observation_sync_id,
        crate::sync::OP_UPSERT,
        &payload,
        project,
    )
}

/// Queues the history a memory holds, under `project`.
///
/// After the memory, which a peer applies in queue order, and only for the
/// memories a project still holds: the live journal queues a version beside the
/// write that replaced it, so a project enrolled afterwards arrived without a
/// single one of its histories, and so did a project a merge had moved the
/// memories into, whose peers read nothing queued under the old name.
pub(super) fn enqueue_stored_versions_tx(
    tx: &Transaction<'_>,
    observation_sync_id: &str,
    project: &str,
) -> Result<(), StoreError> {
    let versions = {
        let mut statement = tx.prepare(
            "SELECT revision, title, content, replaced_at FROM observation_versions
             WHERE observation_sync_id = ?1 ORDER BY revision",
        )?;
        let rows = statement.query_map([observation_sync_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (revision, title, content, replaced_at) in versions {
        enqueue_observation_version(
            tx,
            observation_sync_id,
            revision,
            &title,
            &content,
            &replaced_at,
            project,
        )?;
    }
    Ok(())
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
        enqueue_stored_versions_tx(tx, &observation.sync_id, project)?;
    }
    let prompts = query_column(
        tx,
        "SELECT id FROM prompts WHERE LOWER(ifnull(project, '')) = ?1",
        project,
    )?;
    for id in prompts {
        let prompt = get_prompt_tx(tx, id)?;
        enqueue_session_for_member_tx(tx, &prompt.session_id, &prompt.project)?;
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

/// Hands what a project still had waiting to be sent to the project that took
/// its rows over, once the merge has queued those rows again under the new name.
///
/// The merge used to delete every unsent mutation of the emptied name, on the
/// argument that each row they described had just been queued again. That holds
/// for an upsert of a live row and for nothing else: a deleted memory is not
/// queued again, nor is a judged relation, nor a superseded version, so the one
/// copy of each that existed was thrown away and the peer kept a memory its
/// owner had deleted, with no report of it.
///
/// What is dropped is only an upsert of a memory or prompt that has a newer
/// unsent upsert under the canonical name, which is exactly what the merge
/// queued a moment ago. Everything else is rewritten in place, in its own
/// position in the queue, so a peer still applies a row's history in order.
///
/// A session upsert is kept even when the merge queued it again. The memories
/// and prompts that are carried point at it, and the queued copy now sits after
/// them: dropping the old one made a peer meet a deleted memory's upsert before
/// its session and fail on the foreign key at every pull from then on.
///
/// A delete is rewritten to the canonical project and not left under the old
/// one. A peer ignores a delete whose project is not the one the row is filed
/// under (`apply_observation_delete_tx`), and the peers this is routed to are
/// the ones replicating the canonical project. That guard is what stops a delete
/// from killing a memory that was moved into the project, so it stays; what
/// makes the rewritten delete find a peer's row is the rename the merge queued
/// ahead of it (`enqueue_project_rename_tx`). A delete that was already waiting
/// sits earlier in the queue than that rename, and a peer that has not merged
/// would meet it while still holding the row under the old name and ignore it for
/// good. So a delete is queued again, after the rename, rather than rewritten in
/// place; unless something later is waiting for the same row, whose order it
/// would then upset.
///
/// A canonical name nobody replicates has nowhere to send them, and the unsent
/// mutations go as they always did.
pub(super) fn carry_unsent_mutations_tx(
    tx: &Transaction<'_>,
    source: &str,
    canonical: &str,
) -> Result<(), StoreError> {
    if !is_enrolled_tx(tx, canonical)? {
        tx.execute(
            "DELETE FROM sync_mutations WHERE project = ?1 AND acked_at IS NULL",
            [source],
        )?;
        return Ok(());
    }
    tx.execute(
        "DELETE FROM sync_mutations
          WHERE project = ?1 AND acked_at IS NULL AND op = ?3
            AND entity IN ('observation', 'prompt')
            AND EXISTS (
                SELECT 1 FROM sync_mutations newer
                 WHERE newer.project = ?2 AND newer.acked_at IS NULL
                   AND newer.entity = sync_mutations.entity
                   AND newer.entity_key = sync_mutations.entity_key
                   AND newer.op = ?3 AND newer.seq > sync_mutations.seq)",
        params![source, canonical, crate::sync::OP_UPSERT],
    )?;
    // A version is told apart by its revision, which the key does not carry: the
    // merge queued every version of a live memory again, and the copy that was
    // still waiting is the duplicate.
    tx.execute(
        "DELETE FROM sync_mutations
          WHERE project = ?1 AND acked_at IS NULL AND op = ?3 AND entity = ?4
            AND EXISTS (
                SELECT 1 FROM sync_mutations newer
                 WHERE newer.project = ?2 AND newer.acked_at IS NULL
                   AND newer.entity = sync_mutations.entity
                   AND newer.entity_key = sync_mutations.entity_key
                   AND newer.op = ?3 AND newer.seq > sync_mutations.seq
                   AND json_extract(newer.payload, '$.revision')
                       = json_extract(sync_mutations.payload, '$.revision'))",
        params![
            source,
            canonical,
            crate::sync::OP_UPSERT,
            crate::sync::ENTITY_OBSERVATION_VERSION
        ],
    )?;
    let deletes = {
        let mut statement = tx.prepare(
            "SELECT seq, entity, entity_key, payload FROM sync_mutations
              WHERE project = ?1 AND acked_at IS NULL AND op = ?2
                AND NOT EXISTS (
                    SELECT 1 FROM sync_mutations later
                     WHERE later.project = ?1 AND later.acked_at IS NULL
                       AND later.entity = sync_mutations.entity
                       AND later.entity_key = sync_mutations.entity_key
                       AND later.seq > sync_mutations.seq)
              ORDER BY seq",
        )?;
        let rows = statement.query_map(params![source, crate::sync::OP_DELETE], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for (seq, ..) in &deletes {
        tx.execute("DELETE FROM sync_mutations WHERE seq = ?1", [seq])?;
    }
    // The payload names its project too, and a peer reads that one.
    let moved = tx.execute(
        "UPDATE sync_mutations
            SET project = ?2,
                payload = CASE
                    WHEN json_valid(payload) AND json_type(payload, '$.project') IS NOT NULL
                    THEN json_set(payload, '$.project', ?2)
                    ELSE payload END
          WHERE project = ?1 AND acked_at IS NULL",
        params![source, canonical],
    )?;
    if moved > 0 {
        let project_target = format!("cloud:{canonical}");
        for target in ["cloud", project_target.as_str()] {
            tx.execute(ENSURE_SYNC_TARGET, [target])?;
            tx.execute(
                "UPDATE sync_state
                    SET lifecycle = 'pending', updated_at = datetime('now'),
                        last_enqueued_seq = MAX(last_enqueued_seq,
                            (SELECT MAX(seq) FROM sync_mutations WHERE project = ?2))
                  WHERE target_key = ?1",
                params![target, canonical],
            )?;
        }
    }
    for (_, entity, entity_key, payload) in deletes {
        let mut payload: serde_json::Value = serde_json::from_str(&payload)?;
        if let Some(object) = payload.as_object_mut()
            && object.contains_key("project")
        {
            object.insert("project".to_owned(), canonical.into());
        }
        enqueue_serialized_mutation(
            tx,
            &entity,
            &entity_key,
            crate::sync::OP_DELETE,
            &serde_json::to_string(&payload)?,
            canonical,
        )?;
    }
    Ok(())
}

/// Moves the enrolment of any of `names` to `canonical`, when `canonical` has
/// none. Whether it did.
///
/// Before anything is queued under the new name, which nothing is for a name
/// nobody replicates; see the comment where the merge calls it.
pub(super) fn move_enrolment_tx(
    tx: &Transaction<'_>,
    names: &BTreeSet<String>,
    canonical: &str,
) -> Result<bool, StoreError> {
    if is_enrolled_tx(tx, canonical)? {
        return Ok(false);
    }
    for name in names {
        if is_enrolled_tx(tx, name)? {
            tx.execute(
                "INSERT OR IGNORE INTO sync_enrolled_projects (project) VALUES (?1)",
                [canonical],
            )?;
            return Ok(true);
        }
    }
    Ok(false)
}

/// Files every row of any of `names` under `canonical`. How many observations,
/// sessions and prompts moved.
pub(super) fn reassign_project_rows_tx(
    tx: &Transaction<'_>,
    names: &BTreeSet<String>,
    canonical: &str,
) -> Result<(i64, i64, i64), StoreError> {
    let (mut observations, mut sessions, mut prompts) = (0, 0, 0);
    for name in names {
        observations += tx.execute(
            "UPDATE observations SET project = ?1, updated_at = datetime('now')
             WHERE LOWER(TRIM(ifnull(project, ''))) = ?2",
            params![canonical, name],
        )? as i64;
        sessions += tx.execute(
            "UPDATE sessions SET project = ?1 WHERE LOWER(TRIM(project)) = ?2",
            params![canonical, name],
        )? as i64;
        prompts += tx.execute(
            "UPDATE prompts SET project = ?1
             WHERE LOWER(TRIM(ifnull(project, ''))) = ?2",
            params![canonical, name],
        )? as i64;
    }
    Ok((observations, sessions, prompts))
}

/// Stops replicating the names a merge emptied, and hands what they still had
/// waiting to `canonical`. See [`carry_unsent_mutations_tx`].
pub(super) fn retire_project_names_tx(
    tx: &Transaction<'_>,
    names: &BTreeSet<String>,
    canonical: &str,
) -> Result<(), StoreError> {
    for name in names {
        tx.execute(
            "DELETE FROM sync_enrolled_projects WHERE project = ?1",
            [name],
        )?;
        carry_unsent_mutations_tx(tx, name, canonical)?;
    }
    Ok(())
}

/// Tells a peer that `names` are now called `canonical`.
///
/// A mutation filed under a project is applied to the row the peer holds under
/// that project, and a peer that has not merged holds every row of the old name
/// under the old name: a delete or an update filed under the new one finds
/// nothing there. Saying it once, ahead of the rows the merge queues again, lets
/// every mutation after it find its row, and does not depend on which of those
/// rows the merge happened to queue. A deleted memory is not one of them.
///
/// One per source, keyed by a fresh identifier: a merge is an event and not a
/// state, and two merges of the same names are two things to apply.
pub(super) fn enqueue_project_rename_tx(
    tx: &Transaction<'_>,
    names: &BTreeSet<String>,
    canonical: &str,
) -> Result<(), StoreError> {
    let key: String = tx.query_row(
        "SELECT 'rename-' || lower(hex(randomblob(16)))",
        [],
        |row| row.get(0),
    )?;
    enqueue_mutation(
        tx,
        crate::sync::ENTITY_PROJECT_RENAME,
        &key,
        crate::sync::OP_UPSERT,
        &serde_json::json!({
            "sync_id": key,
            "sources": names,
            "project": canonical,
        }),
        canonical,
    )
}

/// Applies a rename a peer sent: the merge, without the queueing.
///
/// The rows move and the emptied names stop being replicated; nothing is
/// queued, because the peer that merged already queued every row it needed to
/// and a second copy from here would only come back to it.
///
/// A store skips the rename it sent itself, which the pull hands back: by then
/// the old name may be in use again, and moving its rows into the new one would
/// undo what its owner did since. The journal is what says whose it is.
///
/// The names are the sender's word, and only the ones this store replicates are
/// moved: a project that exists on this machine alone, never shared, would
/// otherwise be filed under the shared one and queued to every peer from then
/// on. Enrolment is the table and nothing else (an empty table replicates
/// nothing), the same test the applied delete and every queueing path use. A
/// name that is not enrolled is skipped, not refused, so the rest of the pull
/// goes on; a rename that skips every source creates and enrols nothing.
fn apply_project_rename_tx(
    tx: &Transaction<'_>,
    payload: crate::sync::ProjectRenameMutationPayload,
) -> Result<(), StoreError> {
    let canonical = normalize::project(&payload.project);
    let key = payload.sync_id.trim();
    if canonical.is_empty() || key.is_empty() {
        return Err(invalid_parameter(
            "project rename sync payload requires sync_id and project",
        ));
    }
    let sent_here: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_mutations
          WHERE entity = ?1 AND entity_key = ?2 AND source = 'local')",
        params![crate::sync::ENTITY_PROJECT_RENAME, key],
        |row| row.get(0),
    )?;
    if sent_here {
        return Ok(());
    }
    let mut names = BTreeSet::new();
    for name in &payload.sources {
        let name = name.trim().to_lowercase();
        if !name.is_empty() && name != canonical && is_enrolled_tx(tx, &name)? {
            names.insert(name);
        }
    }
    if names.is_empty() {
        return Ok(());
    }
    move_enrolment_tx(tx, &names, &canonical)?;
    reassign_project_rows_tx(tx, &names, &canonical)?;
    retire_project_names_tx(tx, &names, &canonical)
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
