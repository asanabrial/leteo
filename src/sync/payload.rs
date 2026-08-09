use super::*;

/// Rewrites project-owned data and mutation payloads, then emits compact deterministic JSON.
/// Object keys are sorted by `serde_json`; array order is intentionally retained.
pub fn canonicalize_for_project(payload: &[u8], project: &str) -> Result<Vec<u8>> {
    let mut document: Value =
        serde_json::from_slice(payload).map_err(|error| codec_error("decode chunk data", error))?;
    let object = document
        .as_object_mut()
        .ok_or_else(|| invalid("chunk data must be an object"))?;

    let mutation_entries = object.get("mutations").cloned();
    let has_mutation_list = mutation_entries.is_some();
    let session_mutation_keys = mutation_entries
        .as_ref()
        .map(collect_session_mutation_keys)
        .transpose()?
        .unwrap_or_default();
    let required_session_keys = mutation_entries
        .as_ref()
        .map(|entries| collect_required_session_keys(object, entries))
        .transpose()?
        .unwrap_or_default();

    normalize_direct_rows(object, "sessions", project, |row| {
        if !has_mutation_list {
            return true;
        }
        let session_id = string_field(row, "id").trim();
        session_id.is_empty()
            || session_mutation_keys.contains(session_id)
            || required_session_keys.contains(session_id)
    })?;
    normalize_direct_rows(object, "observations", project, |_| true)?;
    normalize_direct_rows(object, "prompts", project, |_| true)?;

    if let Some(entries) = object.get_mut("mutations") {
        if entries.is_null() {
            *entries = Value::Array(Vec::new());
        }
        let items = entries
            .as_array_mut()
            .ok_or_else(|| invalid("mutations must be an array"))?;
        for (index, item) in items.iter_mut().enumerate() {
            let row = item
                .as_object()
                .ok_or_else(|| invalid(format!("mutations[{index}] must be an object")))?;
            *item = normalize_mutation(row.clone(), project)
                .map_err(|error| invalid(format!("mutations[{index}]: {error}")))?;
        }
    }

    sort_json_keys(&mut document);
    serde_json::to_vec(&document).map_err(|error| codec_error("encode chunk data", error))
}

pub(super) fn normalize_mutation_payload(
    entity: &str,
    op: &str,
    payload: &str,
    project: &str,
) -> Result<(String, String)> {
    match entity {
        "session" => {
            let mut body: SessionMutationPayload = decode_mutation_payload(payload)?;
            body.id = body.id.trim().to_owned();
            body.directory = body.directory.trim().to_owned();
            if body.id.is_empty() {
                return Err(invalid("session payload id is required"));
            }
            if op == OP_UPSERT && body.directory.is_empty() {
                return Err(invalid("session payload directory is required for upsert"));
            }
            if op == OP_DELETE {
                body.directory.clear();
                body.started_at.clear();
                body.ended_at = None;
                body.summary = None;
                trim_optional(&mut body.deleted_at, true);
            }
            body.project = project.to_owned();
            let entity_key = body.id.clone();
            encode_mutation_payload(&body).map(|payload| (payload, entity_key))
        }
        "observation" => {
            let mut body: ObservationMutationPayload = decode_mutation_payload(payload)?;
            body.sync_id = body.sync_id.trim().to_owned();
            body.session_id = body.session_id.trim().to_owned();
            if body.sync_id.is_empty() {
                return Err(invalid("observation payload sync_id is required"));
            }
            if op == OP_UPSERT && body.session_id.is_empty() {
                return Err(invalid(
                    "observation payload session_id is required for upsert",
                ));
            }
            if op == OP_UPSERT {
                body.kind = body.kind.trim().to_owned();
                body.title = body.title.trim().to_owned();
                body.content = body.content.trim().to_owned();
                body.scope = body.scope.trim().to_owned();
                for (value, field) in [
                    (&body.kind, "type"),
                    (&body.title, "title"),
                    (&body.content, "content"),
                    (&body.scope, "scope"),
                ] {
                    if value.is_empty() {
                        return Err(invalid(format!(
                            "observation payload {field} is required for upsert"
                        )));
                    }
                }
            }
            body.project = Some(project.to_owned());
            let entity_key = body.sync_id.clone();
            encode_mutation_payload(&body).map(|payload| (payload, entity_key))
        }
        "prompt" => {
            let mut body: PromptMutationPayload = decode_mutation_payload(payload)?;
            body.sync_id = body.sync_id.trim().to_owned();
            body.session_id = body.session_id.trim().to_owned();
            if body.sync_id.is_empty() {
                return Err(invalid("prompt payload sync_id is required"));
            }
            if op == OP_UPSERT && body.session_id.is_empty() {
                return Err(invalid("prompt payload session_id is required for upsert"));
            }
            if op == OP_UPSERT {
                body.content = body.content.trim().to_owned();
                if body.content.is_empty() {
                    return Err(invalid("prompt payload content is required for upsert"));
                }
            }
            body.project = Some(project.to_owned());
            let entity_key = body.sync_id.clone();
            encode_mutation_payload(&body).map(|payload| (payload, entity_key))
        }
        "relation" => {
            let mut body: RelationMutationPayload = decode_mutation_payload(payload)?;
            body.sync_id = body.sync_id.trim().to_owned();
            body.source_id = body.source_id.trim().to_owned();
            body.target_id = body.target_id.trim().to_owned();
            body.relation = body.relation.trim().to_owned();
            body.judgment_status = body.judgment_status.trim().to_owned();
            trim_optional(&mut body.marked_by_actor, false);
            trim_optional(&mut body.marked_by_kind, false);
            for (value, field) in [
                (&body.sync_id, "sync_id"),
                (&body.source_id, "source_id"),
                (&body.target_id, "target_id"),
                (&body.relation, "relation"),
                (&body.judgment_status, "judgment_status"),
            ] {
                if value.is_empty() {
                    return Err(invalid(format!(
                        "relation payload {field} is required for upsert"
                    )));
                }
            }
            if body.marked_by_actor.as_deref().is_none_or(str::is_empty) {
                return Err(invalid(
                    "relation payload marked_by_actor is required for upsert",
                ));
            }
            if body.marked_by_kind.as_deref().is_none_or(str::is_empty) {
                return Err(invalid(
                    "relation payload marked_by_kind is required for upsert",
                ));
            }
            body.project = project.to_owned();
            let entity_key = body.sync_id.clone();
            encode_mutation_payload(&body).map(|payload| (payload, entity_key))
        }
        _ => Err(invalid(format!("unsupported mutation {entity:?}/{op:?}"))),
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct SessionMutationPayload {
    pub(super) id: String,
    pub(super) project: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(super) directory: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(super) started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) summary: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) deleted_at: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) hard_delete: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct ObservationMutationPayload {
    pub(super) sync_id: String,
    pub(super) session_id: String,
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) title: String,
    pub(super) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) project: Option<String>,
    pub(super) scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) topic_key: Option<String>,
    /// Optional so a chunk written before this field existed still parses, and
    /// so an older peer reading a newer chunk simply ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) prompt_sync_id: Option<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub(super) revision_count: i64,
    #[serde(skip_serializing_if = "is_zero")]
    pub(super) duplicate_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) last_seen_at: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(super) created_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(super) updated_at: String,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) deleted_at: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) hard_delete: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct PromptMutationPayload {
    pub(super) sync_id: String,
    pub(super) session_id: String,
    pub(super) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) project: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(super) created_at: String,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) deleted_at: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) hard_delete: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct RelationMutationPayload {
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
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(super) created_at: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(super) updated_at: String,
}
