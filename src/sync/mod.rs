use std::collections::BTreeSet;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

mod payload;

pub use payload::canonicalize_for_project;
use payload::*;

use crate::{
    memory::model::{Observation, Prompt, Session},
    store::StoreError,
};

pub use crate::memory::model::SyncMutation;

pub const MANIFEST_VERSION: u32 = 1;
pub const MAX_UNCOMPRESSED_CHUNK_BYTES: u64 = 64 * 1024 * 1024;

/// The vocabulary a mutation travels under.
///
/// These cross three modules — the store enqueues them, this codec writes and
/// reads them, and the cloud validates them — and they go out on the wire, so
/// a mismatch between any two is not a compile error. It is a mutation the
/// other side rejects or silently declines to apply, which surfaces much later
/// as memories that did not arrive.
pub const OP_UPSERT: &str = "upsert";
pub const OP_DELETE: &str = "delete";

pub const ENTITY_SESSION: &str = "session";
pub const ENTITY_OBSERVATION: &str = "observation";
pub const ENTITY_PROMPT: &str = "prompt";
pub const ENTITY_RELATION: &str = "relation";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: u32,
    #[serde(default)]
    pub chunks: Vec<ManifestChunk>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            chunks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestChunk {
    pub id: String,
    pub created_by: String,
    pub created_at: String,
    pub sessions: usize,
    pub memories: usize,
    pub prompts: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkData {
    #[serde(default, deserialize_with = "null_to_default")]
    pub sessions: Vec<Session>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub observations: Vec<Observation>,
    #[serde(default, deserialize_with = "null_to_default")]
    pub prompts: Vec<Prompt>,
    #[serde(
        default,
        deserialize_with = "null_to_default",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub mutations: Vec<SyncMutation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedChunk {
    pub entry: ManifestChunk,
    pub data: ChunkData,
}

fn null_to_default<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

#[derive(Debug, thiserror::Error)]
pub enum SyncCodecError {
    #[error("{0}")]
    InvalidData(String),
    #[error("store error: {0}")]
    Store(#[from] StoreError),
}

pub type Result<T> = std::result::Result<T, SyncCodecError>;

pub fn encode_manifest(manifest: &Manifest) -> Result<Vec<u8>> {
    serde_json::to_vec(manifest).map_err(|error| codec_error("encode manifest", error))
}

pub fn decode_manifest(data: &[u8]) -> Result<Manifest> {
    serde_json::from_slice(data).map_err(|error| codec_error("decode manifest", error))
}

pub fn encode_chunk(chunk: &ChunkData) -> Result<Vec<u8>> {
    serde_json::to_vec(chunk).map_err(|error| codec_error("encode chunk data", error))
}

pub fn decode_chunk(data: &[u8]) -> Result<ChunkData> {
    serde_json::from_slice(data).map_err(|error| codec_error("decode chunk data", error))
}

pub fn validate_chunk_id(id: &str) -> Result<()> {
    if id.len() == 8
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(invalid(format!(
            "invalid chunk id {id:?}: expected 8 lowercase hexadecimal characters"
        )))
    }
}

pub fn created_by() -> String {
    ["USER", "USERNAME", "COMPUTERNAME", "HOSTNAME"]
        .into_iter()
        .find_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

pub fn chunk_id(payload: &[u8]) -> String {
    let digest = Sha256::digest(payload);
    hex::encode(&digest[..4])
}

fn normalize_direct_rows<F>(
    document: &mut Map<String, Value>,
    key: &str,
    project: &str,
    should_rewrite: F,
) -> Result<()>
where
    F: Fn(&Map<String, Value>) -> bool,
{
    let Some(entries) = document.get_mut(key) else {
        return Ok(());
    };
    if entries.is_null() {
        *entries = Value::Array(Vec::new());
        return Ok(());
    }
    let items = entries
        .as_array_mut()
        .ok_or_else(|| invalid(format!("{key} must be an array")))?;
    for (index, item) in items.iter_mut().enumerate() {
        let row = item
            .as_object_mut()
            .ok_or_else(|| invalid(format!("{key}[{index}] must be an object")))?;
        if should_rewrite(row) {
            row.insert("project".to_owned(), Value::String(project.to_owned()));
        }
    }
    Ok(())
}

fn collect_session_mutation_keys(entries: &Value) -> Result<BTreeSet<String>> {
    if entries.is_null() {
        return Ok(BTreeSet::new());
    }
    let items = entries
        .as_array()
        .ok_or_else(|| invalid("mutations must be an array"))?;
    let mut keys = BTreeSet::new();
    for (index, item) in items.iter().enumerate() {
        let row = item
            .as_object()
            .ok_or_else(|| invalid(format!("mutations[{index}] must be an object")))?;
        let entity = string_field(row, "entity").trim();
        let op = string_field(row, "op").trim();
        if entity != ENTITY_SESSION || !matches!(op, OP_UPSERT | OP_DELETE) {
            continue;
        }
        let entity_key = string_field(row, "entity_key").trim();
        if !entity_key.is_empty() {
            keys.insert(entity_key.to_owned());
        }
        let payload = string_field(row, "payload").trim();
        if let Ok(body) = decode_mutation_payload::<SessionMutationPayload>(payload) {
            let id = body.id.trim();
            if !id.is_empty() {
                keys.insert(id.to_owned());
            }
        }
    }
    Ok(keys)
}

fn collect_direct_session_keys(document: &Map<String, Value>) -> Result<BTreeSet<String>> {
    let mut keys = BTreeSet::new();
    for entity_key in ["observations", "prompts"] {
        let Some(entries) = document.get(entity_key) else {
            continue;
        };
        if entries.is_null() {
            continue;
        }
        let items = entries
            .as_array()
            .ok_or_else(|| invalid(format!("{entity_key} must be an array")))?;
        for (index, item) in items.iter().enumerate() {
            let row = item
                .as_object()
                .ok_or_else(|| invalid(format!("{entity_key}[{index}] must be an object")))?;
            let session_id = string_field(row, "session_id").trim();
            if !session_id.is_empty() {
                keys.insert(session_id.to_owned());
            }
        }
    }
    Ok(keys)
}

fn collect_required_session_keys(
    document: &Map<String, Value>,
    mutation_entries: &Value,
) -> Result<BTreeSet<String>> {
    let mut keys = collect_direct_session_keys(document)?;
    if mutation_entries.is_null() {
        return Ok(keys);
    }
    let items = mutation_entries
        .as_array()
        .ok_or_else(|| invalid("mutations must be an array"))?;
    for (index, item) in items.iter().enumerate() {
        let row = item
            .as_object()
            .ok_or_else(|| invalid(format!("mutations[{index}] must be an object")))?;
        if string_field(row, "op").trim() != OP_UPSERT {
            continue;
        }
        let payload = string_field(row, "payload").trim();
        let session_id = match string_field(row, "entity").trim() {
            "observation" => decode_mutation_payload::<ObservationMutationPayload>(payload)
                .ok()
                .map(|body| body.session_id),
            "prompt" => decode_mutation_payload::<PromptMutationPayload>(payload)
                .ok()
                .map(|body| body.session_id),
            _ => None,
        };
        if let Some(session_id) = session_id {
            let session_id = session_id.trim();
            if !session_id.is_empty() {
                keys.insert(session_id.to_owned());
            }
        }
    }
    Ok(keys)
}

fn normalize_mutation(raw: Map<String, Value>, project: &str) -> Result<Value> {
    let mut mutation: SyncMutation = serde_json::from_value(Value::Object(raw))
        .map_err(|error| codec_error("decode mutation", error))?;
    mutation.entity = mutation.entity.trim().to_owned();
    mutation.entity_key = mutation.entity_key.trim().to_owned();
    mutation.op = mutation.op.trim().to_owned();
    mutation.payload = mutation.payload.trim().to_owned();

    validate_supported_mutation(&mutation.entity, &mutation.op)?;
    if mutation.payload.is_empty() {
        return Err(invalid("payload is required"));
    }

    let (payload, expected_entity_key) =
        normalize_mutation_payload(&mutation.entity, &mutation.op, &mutation.payload, project)?;
    if mutation.entity_key.is_empty() {
        mutation.entity_key.clone_from(&expected_entity_key);
    }
    if mutation.entity_key != expected_entity_key {
        return Err(invalid(format!(
            "entity_key {:?} does not match payload key {:?}",
            mutation.entity_key, expected_entity_key
        )));
    }

    mutation.project = project.to_owned();
    mutation.payload = payload;
    serde_json::to_value(mutation).map_err(|error| codec_error("encode mutation", error))
}

fn validate_supported_mutation(entity: &str, op: &str) -> Result<()> {
    let supported = match entity {
        ENTITY_SESSION | ENTITY_OBSERVATION | ENTITY_PROMPT => matches!(op, OP_UPSERT | OP_DELETE),
        ENTITY_RELATION => op == OP_UPSERT,
        _ => false,
    };
    if supported {
        Ok(())
    } else {
        Err(invalid(format!("unsupported mutation {entity:?}/{op:?}")))
    }
}

fn decode_mutation_payload<T: DeserializeOwned>(payload: &str) -> Result<T> {
    let payload = payload.trim();
    if payload.is_empty() {
        return Err(invalid("decode mutation payload: empty payload"));
    }
    let decoded;
    let json = if payload.starts_with('"') {
        decoded = serde_json::from_str::<String>(payload)
            .map_err(|error| codec_error("decode mutation payload", error))?;
        decoded.as_str()
    } else {
        payload
    };
    serde_json::from_str(json).map_err(|error| codec_error("decode mutation payload", error))
}

fn encode_mutation_payload<T: Serialize>(payload: &T) -> Result<String> {
    serde_json::to_string(payload).map_err(|error| codec_error("encode mutation payload", error))
}

fn string_field<'a>(row: &'a Map<String, Value>, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn sort_json_keys(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(sort_json_keys),
        Value::Object(object) => {
            object.values_mut().for_each(sort_json_keys);
            object.sort_keys();
        }
        _ => {}
    }
}

fn trim_optional(value: &mut Option<String>, empty_to_none: bool) {
    let Some(current) = value else {
        return;
    };
    let trimmed = current.trim().to_owned();
    if empty_to_none && trimmed.is_empty() {
        *value = None;
    } else {
        *current = trimmed;
    }
}

fn codec_error(context: &str, error: serde_json::Error) -> SyncCodecError {
    invalid(format!("{context}: {error}"))
}

fn invalid(message: impl Into<String>) -> SyncCodecError {
    SyncCodecError::InvalidData(message.into())
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &i64) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests;
