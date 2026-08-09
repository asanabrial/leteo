use std::{collections::BTreeSet, time::Duration};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use thiserror::Error;

use crate::{
    memory::model::SyncMutation,
    sync::{
        ChunkData, Manifest, ManifestChunk, canonicalize_for_project, chunk_id, decode_chunk,
        encode_chunk,
    },
};

use super::{MAX_MUTATION_BATCH_SIZE, auth::ManagedToken};

#[derive(Debug, Clone)]
pub struct CloudStore {
    pool: PgPool,
}

#[derive(Debug, Error)]
pub enum CloudStoreError {
    #[error("postgres error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("invalid cloud data: {0}")]
    Invalid(String),
    #[error("cloud chunk not found")]
    ChunkNotFound,
    #[error("cloud chunk id already exists with different data")]
    ChunkConflict,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MutationEntry {
    pub project: String,
    pub entity: String,
    pub entity_key: String,
    pub op: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredMutation {
    pub seq: i64,
    pub project: String,
    pub entity: String,
    pub entity_key: String,
    pub op: String,
    pub payload: Value,
    pub occurred_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedTokenIdentity {
    pub token_id: i64,
    pub principal_id: i64,
    pub kind: String,
    pub display_name: String,
    pub role: String,
    pub enabled: bool,
    pub revoked: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloudStats {
    pub principals: i64,
    pub chunks: i64,
    pub mutations: i64,
    pub paused_projects: i64,
}

#[derive(Debug, Clone)]
pub struct AuditEntry<'a> {
    pub contributor: &'a str,
    pub project: &'a str,
    pub action: &'a str,
    pub outcome: &'a str,
    pub entry_count: usize,
    pub reason_code: Option<&'a str>,
}

impl CloudStore {
    pub async fn connect(
        database_url: &str,
        max_connections: u32,
    ) -> Result<Self, CloudStoreError> {
        let database_url = database_url.trim();
        if database_url.is_empty() {
            return Err(CloudStoreError::Invalid(
                "database URL is required".to_owned(),
            ));
        }
        let pool = PgPoolOptions::new()
            .max_connections(max_connections.max(1))
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Applies the schema, serialized across every process touching this
    /// database.
    ///
    /// `CREATE TABLE IF NOT EXISTS` is not concurrency-safe in PostgreSQL: two
    /// servers starting at the same moment both pass the existence check and
    /// then collide on the system catalog with a duplicate-key error. The
    /// transaction-scoped advisory lock makes the second one wait and then find
    /// the tables already there. It is released on commit or rollback, so a
    /// crashed migration never leaves the lock held.
    pub async fn migrate(&self) -> Result<(), CloudStoreError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(MIGRATION_LOCK_KEY)
            .execute(&mut *transaction)
            .await?;
        for statement in MIGRATIONS {
            sqlx::query(*statement).execute(&mut *transaction).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn health(&self) -> Result<(), CloudStoreError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub async fn read_manifest(&self, project: &str) -> Result<Manifest, CloudStoreError> {
        let project = require_project(project)?;
        let rows = sqlx::query(
            "SELECT chunk_id, created_by, COALESCE(client_created_at, created_at) AS manifest_at,
                    sessions_count, observations_count, prompts_count
             FROM cloud_chunks WHERE project_name = $1
             ORDER BY created_at, chunk_id",
        )
        .bind(project)
        .fetch_all(&self.pool)
        .await?;
        let chunks = rows
            .into_iter()
            .map(|row| {
                let created_at: DateTime<Utc> = row.try_get("manifest_at")?;
                Ok(ManifestChunk {
                    id: row.try_get("chunk_id")?,
                    created_by: row.try_get("created_by")?,
                    created_at: created_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    sessions: usize_from_i32(row.try_get("sessions_count")?),
                    memories: usize_from_i32(row.try_get("observations_count")?),
                    prompts: usize_from_i32(row.try_get("prompts_count")?),
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(Manifest { version: 1, chunks })
    }

    pub async fn write_chunk(
        &self,
        project: &str,
        provided_chunk_id: &str,
        created_by: &str,
        client_created_at: Option<DateTime<Utc>>,
        payload: &[u8],
    ) -> Result<String, CloudStoreError> {
        let project = require_project(project)?;
        let chunk =
            decode_chunk(payload).map_err(|error| CloudStoreError::Invalid(error.to_string()))?;
        let payload_value: Value = serde_json::from_slice(payload)
            .map_err(|error| CloudStoreError::Invalid(error.to_string()))?;
        let canonical_id = chunk_id(payload);
        if !provided_chunk_id.trim().is_empty() && provided_chunk_id.trim() != canonical_id {
            return Err(CloudStoreError::Invalid(format!(
                "chunk id mismatch: expected {canonical_id}"
            )));
        }
        let counts = (
            i32_from_usize(chunk.sessions.len())?,
            i32_from_usize(chunk.observations.len())?,
            i32_from_usize(chunk.prompts.len())?,
        );
        let mut transaction = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO cloud_chunks
             (project_name, chunk_id, created_by, client_created_at, payload,
              sessions_count, observations_count, prompts_count)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (project_name, chunk_id) DO NOTHING
             RETURNING chunk_id",
        )
        .bind(&project)
        .bind(&canonical_id)
        .bind(nonempty_or(created_by, "unknown"))
        .bind(client_created_at)
        .bind(&payload_value)
        .bind(counts.0)
        .bind(counts.1)
        .bind(counts.2)
        .fetch_optional(&mut *transaction)
        .await?;
        if inserted.is_none() {
            let existing: Value = sqlx::query(
                "SELECT payload FROM cloud_chunks WHERE project_name = $1 AND chunk_id = $2",
            )
            .bind(&project)
            .bind(&canonical_id)
            .fetch_one(&mut *transaction)
            .await?
            .try_get("payload")?;
            if existing != payload_value {
                return Err(CloudStoreError::ChunkConflict);
            }
            transaction.commit().await?;
            return Ok(canonical_id);
        }
        index_chunk_sessions(&mut transaction, &project, &chunk).await?;
        let mutations = materialize_chunk_mutations(&project, &chunk)?;
        insert_mutations_tx(&mut transaction, &mutations).await?;
        transaction.commit().await?;
        Ok(canonical_id)
    }

    pub async fn read_chunk(
        &self,
        project: &str,
        chunk_id: &str,
    ) -> Result<Vec<u8>, CloudStoreError> {
        let project = require_project(project)?;
        let row = sqlx::query(
            "SELECT payload FROM cloud_chunks WHERE project_name = $1 AND chunk_id = $2",
        )
        .bind(project)
        .bind(chunk_id.trim())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(CloudStoreError::ChunkNotFound)?;
        let payload: Value = row.try_get("payload")?;
        serde_json::to_vec(&payload).map_err(|error| CloudStoreError::Invalid(error.to_string()))
    }

    pub async fn known_session_ids(
        &self,
        project: &str,
    ) -> Result<BTreeSet<String>, CloudStoreError> {
        let project = require_project(project)?;
        let rows = sqlx::query(
            "SELECT session_id FROM cloud_project_sessions WHERE project_name = $1 ORDER BY session_id",
        )
        .bind(project)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| row.try_get("session_id"))
            .collect::<Result<_, _>>()
            .map_err(CloudStoreError::from)
    }

    pub async fn insert_mutations(
        &self,
        entries: &[MutationEntry],
    ) -> Result<Vec<i64>, CloudStoreError> {
        validate_mutation_batch(entries)?;
        let mut transaction = self.pool.begin().await?;
        let sequences = insert_mutations_tx(&mut transaction, entries).await?;
        materialize_mutation_chunks(&mut transaction, entries).await?;
        transaction.commit().await?;
        Ok(sequences)
    }

    pub async fn list_mutations_since(
        &self,
        since_sequence: i64,
        limit: usize,
        allowed_projects: Option<&[String]>,
    ) -> Result<(Vec<StoredMutation>, bool, i64), CloudStoreError> {
        let limit = limit.clamp(1, MAX_MUTATION_BATCH_SIZE);
        if allowed_projects.is_some_and(<[String]>::is_empty) {
            return Ok((Vec::new(), false, since_sequence));
        }
        let rows = match allowed_projects {
            None => {
                sqlx::query(
                    "SELECT seq, project, entity, entity_key, op, payload, occurred_at
                     FROM cloud_mutations WHERE seq > $1 ORDER BY seq LIMIT $2",
                )
                .bind(since_sequence.max(0))
                .bind(i64_from_usize(limit + 1)?)
                .fetch_all(&self.pool)
                .await?
            }
            Some(projects) => {
                sqlx::query(
                    "SELECT seq, project, entity, entity_key, op, payload, occurred_at
                     FROM cloud_mutations
                     WHERE seq > $1 AND project = ANY($2)
                     ORDER BY seq LIMIT $3",
                )
                .bind(since_sequence.max(0))
                .bind(projects)
                .bind(i64_from_usize(limit + 1)?)
                .fetch_all(&self.pool)
                .await?
            }
        };
        let mut mutations = rows
            .into_iter()
            .map(map_stored_mutation)
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = mutations.len() > limit;
        mutations.truncate(limit);
        let latest_sequence = mutations
            .last()
            .map_or(since_sequence.max(0), |mutation| mutation.seq);
        Ok((mutations, has_more, latest_sequence))
    }

    pub async fn is_project_sync_enabled(&self, project: &str) -> Result<bool, CloudStoreError> {
        let project = require_project(project)?;
        let row = sqlx::query("SELECT sync_enabled FROM cloud_project_controls WHERE project = $1")
            .bind(project)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(row) => Ok(row.try_get("sync_enabled")?),
            None => Ok(true),
        }
    }

    pub async fn set_project_sync_enabled(
        &self,
        project: &str,
        enabled: bool,
        updated_by: &str,
        reason: Option<&str>,
    ) -> Result<(), CloudStoreError> {
        let project = require_project(project)?;
        sqlx::query(
            "INSERT INTO cloud_project_controls
             (project, sync_enabled, paused_reason, updated_by, updated_at)
             VALUES ($1, $2, $3, $4, NOW())
             ON CONFLICT(project) DO UPDATE SET
                 sync_enabled = EXCLUDED.sync_enabled,
                 paused_reason = EXCLUDED.paused_reason,
                 updated_by = EXCLUDED.updated_by,
                 updated_at = NOW()",
        )
        .bind(project)
        .bind(enabled)
        .bind(reason.map(str::trim).filter(|reason| !reason.is_empty()))
        .bind(nonempty_or(updated_by, "operator"))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_audit(&self, entry: AuditEntry<'_>) -> Result<(), CloudStoreError> {
        sqlx::query(
            "INSERT INTO cloud_sync_audit_log
             (contributor, project, action, outcome, entry_count, reason_code)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(nonempty_or(entry.contributor, "unknown"))
        .bind(require_project(entry.project)?)
        .bind(entry.action.trim())
        .bind(entry.outcome.trim())
        .bind(i32_from_usize(entry.entry_count)?)
        .bind(entry.reason_code)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn stats(&self) -> Result<CloudStats, CloudStoreError> {
        let row = sqlx::query(
            "SELECT
                (SELECT COUNT(*) FROM cloud_human_users) AS users,
                (SELECT COUNT(*) FROM cloud_principals) AS principals,
                (SELECT COUNT(*) FROM cloud_chunks) AS chunks,
                (SELECT COUNT(*) FROM cloud_mutations) AS mutations,
                (SELECT COUNT(*) FROM cloud_project_controls WHERE NOT sync_enabled) AS paused_projects",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(CloudStats {
            principals: row.try_get("principals")?,
            chunks: row.try_get("chunks")?,
            mutations: row.try_get("mutations")?,
            paused_projects: row.try_get("paused_projects")?,
        })
    }

    pub async fn create_principal(
        &self,
        kind: &str,
        display_name: &str,
        role: &str,
    ) -> Result<i64, CloudStoreError> {
        let kind = kind.trim();
        let role = role.trim();
        if !matches!(kind, "human" | "service_account") {
            return Err(CloudStoreError::Invalid(
                "invalid principal kind".to_owned(),
            ));
        }
        if !matches!(role, "admin" | "member") || display_name.trim().is_empty() {
            return Err(CloudStoreError::Invalid("invalid principal".to_owned()));
        }
        let row = sqlx::query(
            "INSERT INTO cloud_principals (kind, display_name, role)
             VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(kind)
        .bind(display_name.trim())
        .bind(role)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.try_get("id")?)
    }

    pub async fn create_user(
        &self,
        principal_id: i64,
        username: &str,
        email: Option<&str>,
    ) -> Result<(), CloudStoreError> {
        if username.trim().is_empty() {
            return Err(CloudStoreError::Invalid("username is required".to_owned()));
        }
        sqlx::query(
            "INSERT INTO cloud_human_users (principal_id, username, email)
             VALUES ($1, $2, $3)",
        )
        .bind(principal_id)
        .bind(username.trim())
        .bind(email.map(str::trim).filter(|email| !email.is_empty()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn store_managed_token(
        &self,
        principal_id: i64,
        token: &ManagedToken,
        verifier: &str,
        name: &str,
    ) -> Result<i64, CloudStoreError> {
        let row = sqlx::query(
            "INSERT INTO cloud_principal_tokens
             (principal_id, token_prefix, token_hash, name)
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(principal_id)
        .bind(&token.prefix)
        .bind(verifier.trim())
        .bind(name.trim())
        .fetch_one(&self.pool)
        .await?;
        Ok(row.try_get("id")?)
    }

    pub async fn grant_project(
        &self,
        principal_id: i64,
        project: &str,
    ) -> Result<(), CloudStoreError> {
        let project = if project.trim() == "*" {
            "*".to_owned()
        } else {
            require_project(project)?
        };
        sqlx::query(
            "INSERT INTO cloud_project_grants (principal_id, project)
             VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(principal_id)
        .bind(project)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Removes a project grant. Used by operators to withdraw access without
    /// deleting the principal.
    pub async fn revoke_project_grant(
        &self,
        principal_id: i64,
        project: &str,
    ) -> Result<bool, CloudStoreError> {
        let project = if project.trim() == "*" {
            "*".to_owned()
        } else {
            require_project(project)?
        };
        let result = sqlx::query(
            "DELETE FROM cloud_project_grants WHERE principal_id = $1 AND project = $2",
        )
        .bind(principal_id)
        .bind(project)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Resolves a principal by its display name so operators do not have to
    /// look identifiers up by hand.
    pub async fn find_principal_by_name(
        &self,
        display_name: &str,
    ) -> Result<Option<i64>, CloudStoreError> {
        let display_name = display_name.trim();
        if display_name.is_empty() {
            return Err(CloudStoreError::Invalid(
                "principal name is required".to_owned(),
            ));
        }
        let row = sqlx::query("SELECT id FROM cloud_principals WHERE display_name = $1")
            .bind(display_name)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| row.try_get("id"))
            .transpose()
            .map_err(Into::into)
    }

    pub async fn find_managed_token(
        &self,
        verifier: &str,
    ) -> Result<Option<ManagedTokenIdentity>, CloudStoreError> {
        let row = sqlx::query(
            "SELECT t.id AS token_id, p.id AS principal_id, p.kind, p.display_name,
                    p.role, p.enabled, t.revoked_at IS NOT NULL AS revoked
             FROM cloud_principal_tokens t
             JOIN cloud_principals p ON p.id = t.principal_id
             WHERE t.token_hash = $1",
        )
        .bind(verifier.trim())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| -> Result<ManagedTokenIdentity, sqlx::Error> {
            Ok(ManagedTokenIdentity {
                token_id: row.try_get("token_id")?,
                principal_id: row.try_get("principal_id")?,
                kind: row.try_get("kind")?,
                display_name: row.try_get("display_name")?,
                role: row.try_get("role")?,
                enabled: row.try_get("enabled")?,
                revoked: row.try_get("revoked")?,
            })
        })
        .transpose()
        .map_err(CloudStoreError::from)
    }

    pub async fn touch_managed_token(&self, token_id: i64) -> Result<(), CloudStoreError> {
        sqlx::query("UPDATE cloud_principal_tokens SET last_used_at = NOW() WHERE id = $1")
            .bind(token_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn principal_has_project_grant(
        &self,
        principal_id: i64,
        project: &str,
    ) -> Result<bool, CloudStoreError> {
        let project = require_project(project)?;
        let row = sqlx::query(
            "SELECT EXISTS(
                 SELECT 1 FROM cloud_project_grants
                 WHERE principal_id = $1 AND project IN ($2, '*')
             ) AS allowed",
        )
        .bind(principal_id)
        .bind(project)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.try_get("allowed")?)
    }

    pub async fn list_principal_project_grants(
        &self,
        principal_id: i64,
    ) -> Result<Vec<String>, CloudStoreError> {
        let rows = sqlx::query(
            "SELECT project FROM cloud_project_grants WHERE principal_id = $1 ORDER BY project",
        )
        .bind(principal_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| row.try_get("project"))
            .collect::<Result<_, _>>()
            .map_err(CloudStoreError::from)
    }

    pub async fn dashboard_session_valid(
        &self,
        principal_id: i64,
        token_id: i64,
    ) -> Result<bool, CloudStoreError> {
        let row = sqlx::query(
            "SELECT EXISTS(
                 SELECT 1
                 FROM cloud_principals p
                 JOIN cloud_principal_tokens t ON t.principal_id = p.id
                 WHERE p.id = $1 AND t.id = $2 AND p.enabled = TRUE
                   AND p.role = 'admin' AND t.revoked_at IS NULL
             ) AS valid",
        )
        .bind(principal_id)
        .bind(token_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.try_get("valid")?)
    }
}

fn require_project(project: &str) -> Result<String, CloudStoreError> {
    let project = crate::memory::normalize::project(project);
    if project.is_empty() {
        Err(CloudStoreError::Invalid("project is required".to_owned()))
    } else {
        Ok(project)
    }
}

fn validate_mutation_batch(entries: &[MutationEntry]) -> Result<(), CloudStoreError> {
    if entries.is_empty() {
        return Err(CloudStoreError::Invalid(
            "mutation batch cannot be empty".to_owned(),
        ));
    }
    if entries.len() > MAX_MUTATION_BATCH_SIZE {
        return Err(CloudStoreError::Invalid(format!(
            "mutation batch cannot exceed {MAX_MUTATION_BATCH_SIZE} entries"
        )));
    }
    for entry in entries {
        validate_mutation_entry(entry)?;
    }
    Ok(())
}

fn validate_mutation_entry(entry: &MutationEntry) -> Result<(), CloudStoreError> {
    require_project(&entry.project)?;
    if entry.entity.trim().is_empty()
        || entry.entity_key.trim().is_empty()
        || !matches!(
            entry.op.trim(),
            crate::sync::OP_UPSERT | crate::sync::OP_DELETE
        )
    {
        return Err(CloudStoreError::Invalid(
            "mutation requires entity, entity_key, and a supported operation".to_owned(),
        ));
    }
    Ok(())
}

async fn insert_mutations_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    entries: &[MutationEntry],
) -> Result<Vec<i64>, CloudStoreError> {
    let mut sequences = Vec::with_capacity(entries.len());
    for entry in entries {
        let row = sqlx::query(
            "INSERT INTO cloud_mutations (project, entity, entity_key, op, payload)
             VALUES ($1, $2, $3, $4, $5) RETURNING seq",
        )
        .bind(&entry.project)
        .bind(entry.entity.trim())
        .bind(entry.entity_key.trim())
        .bind(entry.op.trim())
        .bind(&entry.payload)
        .fetch_one(&mut **transaction)
        .await?;
        sequences.push(row.try_get("seq")?);
    }
    Ok(sequences)
}

async fn index_chunk_sessions(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    project: &str,
    chunk: &ChunkData,
) -> Result<(), CloudStoreError> {
    let mut session_ids = chunk
        .sessions
        .iter()
        .map(|session| session.id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect::<BTreeSet<_>>();
    for mutation in &chunk.mutations {
        if mutation.entity == crate::sync::ENTITY_SESSION && mutation.op == crate::sync::OP_UPSERT {
            let payload = decode_mutation_payload(&mutation.payload)?;
            if let Some(id) = payload.get("id").and_then(Value::as_str) {
                session_ids.insert(id.trim().to_owned());
            }
        }
    }
    session_ids.remove("");
    for session_id in session_ids {
        sqlx::query(
            "INSERT INTO cloud_project_sessions (project_name, session_id)
             VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(project)
        .bind(session_id)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn materialize_mutation_chunks(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    entries: &[MutationEntry],
) -> Result<(), CloudStoreError> {
    let mut grouped = std::collections::BTreeMap::<String, Vec<&MutationEntry>>::new();
    for entry in entries {
        grouped
            .entry(entry.project.clone())
            .or_default()
            .push(entry);
    }
    for (project, entries) in grouped {
        let chunk = ChunkData {
            mutations: entries
                .into_iter()
                .map(|entry| SyncMutation {
                    entity: entry.entity.clone(),
                    entity_key: entry.entity_key.clone(),
                    op: entry.op.clone(),
                    payload: serde_json::to_string(&entry.payload)
                        .unwrap_or_else(|_| "{}".to_owned()),
                    project: project.clone(),
                    ..SyncMutation::default()
                })
                .collect(),
            ..ChunkData::default()
        };
        let encoded =
            encode_chunk(&chunk).map_err(|error| CloudStoreError::Invalid(error.to_string()))?;
        let canonical = canonicalize_for_project(&encoded, &project)
            .map_err(|error| CloudStoreError::Invalid(error.to_string()))?;
        let payload: Value = serde_json::from_slice(&canonical)
            .map_err(|error| CloudStoreError::Invalid(error.to_string()))?;
        let id = chunk_id(&canonical);
        sqlx::query(
            "INSERT INTO cloud_chunks
             (project_name, chunk_id, created_by, payload,
              sessions_count, observations_count, prompts_count)
             VALUES ($1, $2, 'mutation-push', $3, 0, 0, 0)
             ON CONFLICT (project_name, chunk_id) DO NOTHING",
        )
        .bind(&project)
        .bind(id)
        .bind(payload)
        .execute(&mut **transaction)
        .await?;
        let canonical_chunk = decode_chunk(&canonical)
            .map_err(|error| CloudStoreError::Invalid(error.to_string()))?;
        index_chunk_sessions(transaction, &project, &canonical_chunk).await?;
    }
    Ok(())
}

fn materialize_chunk_mutations(
    project: &str,
    chunk: &ChunkData,
) -> Result<Vec<MutationEntry>, CloudStoreError> {
    let mut entries = Vec::new();
    for session in &chunk.sessions {
        entries.push(MutationEntry {
            project: project.to_owned(),
            entity: "session".to_owned(),
            entity_key: session.id.trim().to_owned(),
            op: crate::sync::OP_UPSERT.to_owned(),
            payload: serde_json::to_value(session)
                .map_err(|error| CloudStoreError::Invalid(error.to_string()))?,
        });
    }
    for observation in &chunk.observations {
        entries.push(MutationEntry {
            project: project.to_owned(),
            entity: "observation".to_owned(),
            entity_key: observation.sync_id.trim().to_owned(),
            op: crate::sync::OP_UPSERT.to_owned(),
            payload: serde_json::to_value(observation)
                .map_err(|error| CloudStoreError::Invalid(error.to_string()))?,
        });
    }
    for prompt in &chunk.prompts {
        entries.push(MutationEntry {
            project: project.to_owned(),
            entity: "prompt".to_owned(),
            entity_key: prompt.sync_id.trim().to_owned(),
            op: crate::sync::OP_UPSERT.to_owned(),
            payload: serde_json::to_value(prompt)
                .map_err(|error| CloudStoreError::Invalid(error.to_string()))?,
        });
    }
    for mutation in &chunk.mutations {
        if mutation.entity == crate::sync::ENTITY_RELATION || mutation.op == crate::sync::OP_DELETE
        {
            entries.push(MutationEntry {
                project: project.to_owned(),
                entity: mutation.entity.trim().to_owned(),
                entity_key: mutation.entity_key.trim().to_owned(),
                op: mutation.op.trim().to_owned(),
                payload: decode_mutation_payload(&mutation.payload)?,
            });
        }
    }
    for entry in &entries {
        validate_mutation_entry(entry)?;
    }
    Ok(entries)
}

fn decode_mutation_payload(payload: &str) -> Result<Value, CloudStoreError> {
    let payload = payload.trim();
    let value: Value = serde_json::from_str(payload)
        .map_err(|error| CloudStoreError::Invalid(error.to_string()))?;
    if let Some(encoded) = value.as_str() {
        serde_json::from_str(encoded).map_err(|error| CloudStoreError::Invalid(error.to_string()))
    } else {
        Ok(value)
    }
}

fn map_stored_mutation(row: sqlx::postgres::PgRow) -> Result<StoredMutation, sqlx::Error> {
    let occurred_at: DateTime<Utc> = row.try_get("occurred_at")?;
    Ok(StoredMutation {
        seq: row.try_get("seq")?,
        project: row.try_get("project")?,
        entity: row.try_get("entity")?,
        entity_key: row.try_get("entity_key")?,
        op: row.try_get("op")?,
        payload: row.try_get("payload")?,
        occurred_at: occurred_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    })
}

fn nonempty_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value.trim()
    }
}

fn i32_from_usize(value: usize) -> Result<i32, CloudStoreError> {
    i32::try_from(value).map_err(|_| CloudStoreError::Invalid("count is too large".to_owned()))
}

fn i64_from_usize(value: usize) -> Result<i64, CloudStoreError> {
    i64::try_from(value).map_err(|_| CloudStoreError::Invalid("limit is too large".to_owned()))
}

fn usize_from_i32(value: i32) -> usize {
    usize::try_from(value).unwrap_or_default()
}

/// Advisory-lock key that serializes schema migrations. The value is arbitrary
/// but must stay stable: changing it lets an old and a new binary migrate
/// concurrently.
const MIGRATION_LOCK_KEY: i64 = 0x4C_45_54_45_4F_01;

const MIGRATIONS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS cloud_principals (
        id BIGSERIAL PRIMARY KEY,
        kind TEXT NOT NULL CHECK (kind IN ('human', 'service_account')),
        display_name TEXT NOT NULL,
        role TEXT NOT NULL DEFAULT 'member' CHECK (role IN ('admin', 'member')),
        enabled BOOLEAN NOT NULL DEFAULT TRUE,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )",
    "CREATE TABLE IF NOT EXISTS cloud_human_users (
        principal_id BIGINT PRIMARY KEY REFERENCES cloud_principals(id) ON DELETE CASCADE,
        username TEXT NOT NULL UNIQUE,
        email TEXT UNIQUE,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )",
    "CREATE TABLE IF NOT EXISTS cloud_principal_tokens (
        id BIGSERIAL PRIMARY KEY,
        principal_id BIGINT NOT NULL REFERENCES cloud_principals(id) ON DELETE CASCADE,
        token_prefix TEXT NOT NULL,
        token_hash TEXT NOT NULL UNIQUE,
        name TEXT NOT NULL DEFAULT '',
        created_by_principal_id BIGINT REFERENCES cloud_principals(id),
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        last_used_at TIMESTAMPTZ,
        revoked_at TIMESTAMPTZ,
        revoked_by_principal_id BIGINT REFERENCES cloud_principals(id),
        revocation_reason TEXT
    )",
    "CREATE INDEX IF NOT EXISTS idx_cloud_principal_tokens_principal
     ON cloud_principal_tokens(principal_id, revoked_at)",
    "CREATE TABLE IF NOT EXISTS cloud_project_grants (
        principal_id BIGINT NOT NULL REFERENCES cloud_principals(id) ON DELETE CASCADE,
        project TEXT NOT NULL,
        granted_by_principal_id BIGINT REFERENCES cloud_principals(id),
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        PRIMARY KEY (principal_id, project)
    )",
    "CREATE TABLE IF NOT EXISTS cloud_chunks (
        project_name TEXT NOT NULL,
        chunk_id TEXT NOT NULL,
        created_by TEXT NOT NULL,
        client_created_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        payload JSONB NOT NULL,
        sessions_count INTEGER NOT NULL DEFAULT 0,
        observations_count INTEGER NOT NULL DEFAULT 0,
        prompts_count INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (project_name, chunk_id)
    )",
    "CREATE TABLE IF NOT EXISTS cloud_project_sessions (
        project_name TEXT NOT NULL,
        session_id TEXT NOT NULL,
        indexed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        PRIMARY KEY (project_name, session_id)
    )",
    "CREATE TABLE IF NOT EXISTS cloud_mutations (
        seq BIGSERIAL PRIMARY KEY,
        project TEXT NOT NULL,
        entity TEXT NOT NULL,
        entity_key TEXT NOT NULL,
        op TEXT NOT NULL CHECK (op IN ('upsert', 'delete')),
        payload JSONB NOT NULL DEFAULT '{}'::jsonb,
        occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )",
    "CREATE INDEX IF NOT EXISTS idx_cloud_mutations_project_seq
     ON cloud_mutations(project, seq)",
    "CREATE TABLE IF NOT EXISTS cloud_project_controls (
        project TEXT PRIMARY KEY,
        sync_enabled BOOLEAN NOT NULL DEFAULT TRUE,
        paused_reason TEXT,
        updated_by TEXT,
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
    )",
    "CREATE TABLE IF NOT EXISTS cloud_sync_audit_log (
        id BIGSERIAL PRIMARY KEY,
        occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        contributor TEXT NOT NULL,
        project TEXT NOT NULL,
        action TEXT NOT NULL,
        outcome TEXT NOT NULL,
        entry_count INTEGER NOT NULL DEFAULT 0,
        reason_code TEXT,
        metadata JSONB
    )",
    "CREATE INDEX IF NOT EXISTS idx_cloud_sync_audit_occurred
     ON cloud_sync_audit_log(occurred_at DESC)",
    "CREATE TABLE IF NOT EXISTS cloud_auth_audit_log (
        id BIGSERIAL PRIMARY KEY,
        occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        actor_principal_id BIGINT REFERENCES cloud_principals(id),
        actor_source TEXT NOT NULL,
        target_principal_id BIGINT REFERENCES cloud_principals(id),
        project TEXT,
        action TEXT NOT NULL,
        outcome TEXT NOT NULL,
        reason_code TEXT,
        metadata JSONB
    )",
];

#[cfg(test)]
mod tests;
