use super::*;

pub(super) fn validate_chunk_payload(payload: &[u8]) -> Result<ChunkData, ApiError> {
    let chunk = decode_chunk(payload).map_err(|error| ApiError::bad_request(error.to_string()))?;
    for (index, session) in chunk.sessions.iter().enumerate() {
        if session.id.trim().is_empty() || session.directory.trim().is_empty() {
            return Err(ApiError::bad_request(format!(
                "sessions[{index}] requires id and directory"
            )));
        }
    }
    for (index, observation) in chunk.observations.iter().enumerate() {
        if observation.sync_id.trim().is_empty()
            || observation.session_id.trim().is_empty()
            || observation.kind.trim().is_empty()
            || observation.title.trim().is_empty()
            || observation.content.trim().is_empty()
            || observation.scope.trim().is_empty()
        {
            return Err(ApiError::bad_request(format!(
                "observations[{index}] is missing required fields"
            )));
        }
    }
    for (index, prompt) in chunk.prompts.iter().enumerate() {
        if prompt.sync_id.trim().is_empty()
            || prompt.session_id.trim().is_empty()
            || prompt.content.trim().is_empty()
        {
            return Err(ApiError::bad_request(format!(
                "prompts[{index}] is missing required fields"
            )));
        }
    }
    let entries = chunk
        .mutations
        .iter()
        .map(|mutation| {
            Ok(MutationEntry {
                project: mutation.project.clone(),
                entity: mutation.entity.clone(),
                entity_key: mutation.entity_key.clone(),
                op: mutation.op.clone(),
                payload: decode_mutation_payload(&mutation.payload)?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    if !entries.is_empty() {
        validate_mutation_entries(&entries)?;
    }
    Ok(chunk)
}

pub(super) fn validate_mutation_entries(entries: &[MutationEntry]) -> Result<(), ApiError> {
    if entries.is_empty() {
        return Err(ApiError::bad_request(
            "mutation batch must contain at least one entry",
        ));
    }
    if entries.len() > MAX_MUTATION_BATCH_SIZE {
        return Err(ApiError::bad_request(format!(
            "mutation batch cannot exceed {MAX_MUTATION_BATCH_SIZE} entries"
        )));
    }
    for (index, entry) in entries.iter().enumerate() {
        if entry.project.trim().is_empty()
            || entry.entity_key.trim().is_empty()
            || !matches!(
                entry.entity.as_str(),
                "session" | "observation" | "prompt" | "relation"
            )
        {
            return Err(ApiError::bad_request(format!(
                "entries[{index}] has invalid project, entity, or entity_key"
            )));
        }
        let supported_operation = match entry.entity.as_str() {
            crate::sync::ENTITY_RELATION => entry.op == crate::sync::OP_UPSERT,
            _ => matches!(
                entry.op.as_str(),
                crate::sync::OP_UPSERT | crate::sync::OP_DELETE
            ),
        };
        if !supported_operation || !entry.payload.is_object() {
            return Err(ApiError::bad_request(format!(
                "entries[{index}] has invalid operation or payload"
            )));
        }
        // The key a mutation is filed under and the identifier inside it have
        // to name the same thing. This server orders and deduplicates by
        // `entity_key`; every peer applies by the identifier in the payload.
        // One where they disagree is stored and served as being about one
        // memory and applied to another — the same change arriving twice, or
        // landing on a memory the server believes untouched.
        //
        // Leteo's own sender builds both from one value, at all five places
        // that queue a mutation, so this only ever catches a client that has
        // drifted from the format.
        let identifier = match entry.entity.as_str() {
            crate::sync::ENTITY_SESSION => "id",
            _ => "sync_id",
        };
        if entry
            .payload
            .get(identifier)
            .and_then(Value::as_str)
            .is_none_or(|value| value.trim() != entry.entity_key.trim())
        {
            return Err(ApiError::bad_request(format!(
                "entries[{index}].payload.{identifier} must be the entity_key {:?}",
                entry.entity_key
            )));
        }
        if entry.entity == "relation" {
            for field in [
                "sync_id",
                "source_id",
                "target_id",
                "relation",
                "judgment_status",
                "marked_by_actor",
                "marked_by_kind",
            ] {
                if entry
                    .payload
                    .get(field)
                    .and_then(Value::as_str)
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(ApiError::bad_request(format!(
                        "entries[{index}].payload.{field} is required"
                    )));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn validate_session_references(
    chunk: &ChunkData,
    known: &BTreeSet<String>,
) -> Result<(), ApiError> {
    let mut sessions = known.clone();
    sessions.extend(
        chunk
            .sessions
            .iter()
            .map(|session| session.id.trim().to_owned()),
    );
    for mutation in &chunk.mutations {
        if mutation.entity == crate::sync::ENTITY_SESSION && mutation.op == crate::sync::OP_UPSERT {
            let payload = decode_mutation_payload(&mutation.payload)?;
            if let Some(id) = payload.get("id").and_then(Value::as_str) {
                sessions.insert(id.trim().to_owned());
            }
        }
    }
    for (label, session_id) in chunk
        .observations
        .iter()
        .map(|item| ("observation", item.session_id.as_str()))
        .chain(
            chunk
                .prompts
                .iter()
                .map(|item| ("prompt", item.session_id.as_str())),
        )
    {
        if !sessions.contains(session_id.trim()) {
            return Err(ApiError::bad_request(format!(
                "{label} references missing session_id {session_id:?}"
            )));
        }
    }
    for mutation in &chunk.mutations {
        if matches!(
            mutation.entity.as_str(),
            crate::sync::ENTITY_OBSERVATION | crate::sync::ENTITY_PROMPT
        ) && mutation.op == crate::sync::OP_UPSERT
        {
            let payload = decode_mutation_payload(&mutation.payload)?;
            let session_id = payload
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            if session_id.is_empty() || !sessions.contains(session_id) {
                return Err(ApiError::bad_request(format!(
                    "mutation references missing session_id {session_id:?}"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn decode_mutation_payload(payload: &str) -> Result<Value, ApiError> {
    let value: Value = serde_json::from_str(payload.trim())
        .map_err(|error| ApiError::bad_request(format!("invalid mutation payload: {error}")))?;
    if let Some(encoded) = value.as_str() {
        serde_json::from_str(encoded)
            .map_err(|error| ApiError::bad_request(format!("invalid mutation payload: {error}")))
    } else {
        Ok(value)
    }
}
