//! The journal and the apply path that keep two stores in step.

use super::*;

impl Store {
    pub fn list_deferred(
        &self,
        options: ListDeferredOptions,
    ) -> Result<Vec<DeferredRow>, StoreError> {
        let status = options.status.unwrap_or_default();
        let limit = options.limit.unwrap_or(usize::MAX).min(i64::MAX as usize) as i64;
        let offset = options.offset.min(i64::MAX as usize) as i64;
        let mut statement = self.connection.prepare(
            "SELECT sync_id, entity, payload, apply_status, retry_count,
                    last_error, last_attempted_at, first_seen_at
             FROM sync_deferred_mutations
             WHERE ?1 = '' OR apply_status = ?1
             ORDER BY datetime(first_seen_at), sync_id
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = statement.query_map(params![status, limit, offset], map_deferred_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn get_deferred(&self, sync_id: &str) -> Result<DeferredRow, StoreError> {
        self.connection
            .query_row(
                "SELECT sync_id, entity, payload, apply_status, retry_count,
                        last_error, last_attempted_at, first_seen_at
                 FROM sync_deferred_mutations WHERE sync_id = ?1",
                [sync_id],
                map_deferred_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::RelationNotFound(sync_id.to_owned()))
    }

    /// Counts mutations still waiting to be pushed to a sync target.
    pub fn pending_sync_mutation_count(&self, target_key: &str) -> Result<i64, StoreError> {
        let target_key = require_sync_target(target_key)?;
        Ok(self.connection.query_row(
            "SELECT COUNT(*) FROM sync_mutations
             WHERE target_key = ?1 AND acked_at IS NULL",
            [&target_key],
            |row| row.get(0),
        )?)
    }

    /// When the oldest of those was written, if any are waiting.
    ///
    /// The count alone does not answer the question somebody asks it: a hundred
    /// pending from this morning is a peer that has been busy, and a hundred
    /// pending since March is replication that stopped and nobody noticed. Only
    /// the age tells them apart, and the queue drains on nothing but an
    /// acknowledgement — `prune_acked_mutations_tx` deletes rows that were
    /// acked and have aged out, so an unreachable peer means the queue keeps
    /// every row it ever took.
    ///
    /// That is the right behaviour for a queue that must not lose a write, and
    /// it is not free: enrolling a project cost 62% more disk per memory on a
    /// measured run — 20 memories of about two kilobytes took 116 KB
    /// unenrolled and 188 KB enrolled, the difference being this journal
    /// carrying each write's payload a second time.
    pub fn oldest_pending_mutation(&self, target_key: &str) -> Result<Option<String>, StoreError> {
        let target_key = require_sync_target(target_key)?;
        Ok(self
            .connection
            .query_row(
                "SELECT MIN(occurred_at) FROM sync_mutations
                 WHERE target_key = ?1 AND acked_at IS NULL",
                [&target_key],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten())
    }

    /// The same state, read without writing anything.
    ///
    /// [`Store::get_sync_state`] runs `ENSURE_SYNC_TARGET` before it reads, so
    /// asking it a question takes the write lock. That is right for the sync
    /// loop, whose next act is to update that row anyway, and wrong for
    /// anything that only wants to look: Leteo is multi-writer by design — the
    /// MCP server, the hooks, the CLI and the background sync all hold this one
    /// file — and the dashboard redraws on every keypress. Reaching for the
    /// write lock to paint a screen is contention bought for nothing.
    ///
    /// `None` for a target that has no row, rather than a default full of
    /// zeroes: that would read as a healthy idle queue, which is the confusion
    /// this whole page has already been fixed for once.
    ///
    /// Nothing tests that the dashboard uses this one rather than the other.
    /// It cannot: `load_cloud` takes `&Store`, so the borrow checker refuses
    /// the writing variant outright, and a test asserting the same thing sat
    /// green through every mutation because the compiler got there first.
    pub fn sync_state_if_any(&self, target_key: &str) -> Result<Option<SyncState>, StoreError> {
        let target_key = require_sync_target(target_key)?;
        self.connection
            .query_row(
                "SELECT target_key, lifecycle, last_enqueued_seq, last_acked_seq,
                    last_pulled_seq, consecutive_failures, backoff_until, lease_owner,
                    lease_until, reason_code, reason_message, last_error, updated_at
             FROM sync_state WHERE target_key = ?1",
                [&target_key],
                map_sync_state,
            )
            .optional()
            .map_err(StoreError::from)
    }

    pub fn get_sync_state(&mut self, target_key: &str) -> Result<SyncState, StoreError> {
        let target_key = require_sync_target(target_key)?;
        self.connection.execute(ENSURE_SYNC_TARGET, [&target_key])?;
        self.connection
            .query_row(
                "SELECT target_key, lifecycle, last_enqueued_seq, last_acked_seq,
                    last_pulled_seq, consecutive_failures, backoff_until, lease_owner,
                    lease_until, reason_code, reason_message, last_error, updated_at
             FROM sync_state WHERE target_key = ?1",
                [&target_key],
                map_sync_state,
            )
            .map_err(StoreError::from)
    }

    pub fn list_pending_sync_mutations(
        &self,
        target_key: &str,
        allowed_projects: &[String],
        limit: usize,
    ) -> Result<Vec<SyncMutation>, StoreError> {
        let target_key = require_sync_target(target_key)?;
        let limit = limit.clamp(1, 100) as i64;
        let allow_all = allowed_projects.iter().any(|project| project.trim() == "*");
        if allowed_projects.is_empty() && !allow_all {
            return Ok(Vec::new());
        }
        let projects = if allow_all {
            "*".to_owned()
        } else {
            serde_json::to_string(
                &allowed_projects
                    .iter()
                    .map(|project| normalize::project(project))
                    .filter(|project| !project.is_empty())
                    .collect::<BTreeSet<_>>(),
            )?
        };
        let mut statement = self.connection.prepare(&format!(
            "SELECT {SYNC_MUTATION_COLUMNS}
             FROM sync_mutations
             WHERE target_key = ?1 AND acked_at IS NULL AND source = 'local'
               AND (?2 = '*' OR project IN (SELECT value FROM json_each(?2)))
             ORDER BY seq LIMIT ?3"
        ))?;
        let rows = statement.query_map(params![target_key, projects, limit], map_sync_mutation)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn ack_sync_mutation_seqs(
        &mut self,
        target_key: &str,
        sequences: &[i64],
    ) -> Result<(), StoreError> {
        let target_key = require_sync_target(target_key)?;
        if sequences.is_empty() {
            return Ok(());
        }
        let tx = self.write_transaction()?;
        let mut last_acked = 0;
        for sequence in sequences.iter().copied().filter(|sequence| *sequence > 0) {
            let changed = tx.execute(
                "UPDATE sync_mutations SET acked_at = datetime('now')
                 WHERE target_key = ?1 AND seq = ?2 AND acked_at IS NULL",
                params![target_key, sequence],
            )?;
            if changed > 0 {
                last_acked = last_acked.max(sequence);
            }
        }
        if last_acked > 0 {
            tx.execute(
                "UPDATE sync_state
                 SET last_acked_seq = MAX(last_acked_seq, ?2), lifecycle = 'idle',
                     updated_at = datetime('now')
                 WHERE target_key = ?1",
                params![target_key, last_acked],
            )?;
        }
        prune_acked_mutations_tx(&tx, &target_key)?;
        tx.commit()?;
        Ok(())
    }

    pub fn acquire_sync_lease(
        &mut self,
        target_key: &str,
        owner: &str,
        ttl: Duration,
        now: chrono::DateTime<Utc>,
    ) -> Result<bool, StoreError> {
        let target_key = require_sync_target(target_key)?;
        let owner = owner.trim();
        if owner.is_empty() {
            return Err(invalid_parameter("sync lease owner is required"));
        }
        let ttl = chrono::Duration::from_std(ttl.max(Duration::from_secs(1)))
            .map_err(|_| invalid_parameter("sync lease duration is too large"))?;
        let now = now.to_rfc3339();
        let lease_until = (chrono::DateTime::parse_from_rfc3339(&now)
            .map_err(|_| invalid_parameter("invalid sync lease time"))?
            + ttl)
            .to_rfc3339();
        let tx = self.write_transaction()?;
        tx.execute(ENSURE_SYNC_TARGET, [&target_key])?;
        let changed = tx.execute(
            "UPDATE sync_state
             SET lease_owner = ?2, lease_until = ?3, updated_at = datetime('now')
             WHERE target_key = ?1
               AND (lease_owner IS NULL OR lease_owner = ?2
                    OR lease_until IS NULL OR datetime(lease_until) <= datetime(?4))",
            params![target_key, owner, lease_until, now],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    pub fn release_sync_lease(&mut self, target_key: &str, owner: &str) -> Result<(), StoreError> {
        let target_key = require_sync_target(target_key)?;
        self.connection.execute(
            "UPDATE sync_state SET lease_owner = NULL, lease_until = NULL,
                 updated_at = datetime('now')
             WHERE target_key = ?1 AND lease_owner = ?2",
            params![target_key, owner.trim()],
        )?;
        Ok(())
    }

    pub fn apply_pulled_sync_mutation(
        &mut self,
        target_key: &str,
        mutation: &SyncMutation,
    ) -> Result<bool, StoreError> {
        let target_key = require_sync_target(target_key)?;
        if mutation.seq <= 0 {
            return Err(invalid_parameter(
                "pulled mutation sequence must be positive",
            ));
        }
        // Captured before the transaction borrows `self`.
        let max_content_bytes = self.config.max_observation_length;
        let tx = self.write_transaction()?;
        tx.execute(ENSURE_SYNC_TARGET, [&target_key])?;
        let cursor = tx.query_row(
            "SELECT last_pulled_seq FROM sync_state WHERE target_key = ?1",
            [&target_key],
            |row| row.get::<_, i64>(0),
        )?;
        if mutation.seq <= cursor {
            return Ok(false);
        }
        apply_sync_mutation_tx(&tx, mutation, max_content_bytes)?;
        replay_deferred_relations_tx(&tx, max_content_bytes)?;
        tx.execute(
            "UPDATE sync_state
             SET last_pulled_seq = ?2, lifecycle = 'pulling', updated_at = datetime('now')
             WHERE target_key = ?1",
            params![target_key, mutation.seq],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn mark_sync_failure(
        &mut self,
        target_key: &str,
        message: &str,
        backoff_until: chrono::DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let target_key = require_sync_target(target_key)?;
        self.connection.execute(
            "INSERT INTO sync_state
             (target_key, lifecycle, consecutive_failures, backoff_until, last_error, updated_at)
             VALUES (?1, 'backoff', 1, ?2, ?3, datetime('now'))
             ON CONFLICT(target_key) DO UPDATE SET
                 lifecycle = 'backoff',
                 consecutive_failures = sync_state.consecutive_failures + 1,
                 backoff_until = excluded.backoff_until,
                 last_error = excluded.last_error,
                 updated_at = datetime('now')",
            params![target_key, backoff_until.to_rfc3339(), message.trim()],
        )?;
        Ok(())
    }

    pub fn mark_sync_healthy(&mut self, target_key: &str) -> Result<(), StoreError> {
        let target_key = require_sync_target(target_key)?;
        self.connection.execute(
            "INSERT INTO sync_state (target_key, lifecycle, updated_at)
             VALUES (?1, 'healthy', datetime('now'))
             ON CONFLICT(target_key) DO UPDATE SET
                 lifecycle = 'healthy', consecutive_failures = 0, backoff_until = NULL,
                 reason_code = NULL, reason_message = NULL, last_error = NULL,
                 updated_at = datetime('now')",
            [&target_key],
        )?;
        Ok(())
    }

    /// Retries a bounded batch of deferred relation mutations. Each item is
    /// applied in its own transaction, so one poisoned payload cannot block the
    /// queue, and rows that keep failing are retired as dead.
    pub fn replay_deferred_sync_mutations(&mut self) -> Result<ReplayDeferredResult, StoreError> {
        // Captured before the transactions below borrow `self`.
        let max_content_bytes = self.config.max_observation_length;
        let pending = {
            let mut statement = self.connection.prepare(
                "SELECT sync_id, payload, retry_count FROM sync_deferred_mutations
                 WHERE entity = 'relation' AND apply_status = 'deferred'
                 ORDER BY datetime(first_seen_at), sync_id
                 LIMIT ?1",
            )?;
            let rows = statement.query_map([DEFERRED_REPLAY_BATCH], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        let mut result = ReplayDeferredResult::default();
        for (sync_id, payload, retry_count) in pending {
            result.retried += 1;
            let mutation = SyncMutation {
                entity: "relation".to_owned(),
                entity_key: sync_id.clone(),
                op: crate::sync::OP_UPSERT.to_owned(),
                payload,
                source: "remote".to_owned(),
                ..SyncMutation::default()
            };
            let tx = self.write_transaction()?;
            match apply_relation_upsert_tx(&tx, &mutation, max_content_bytes) {
                Ok(()) => {
                    let deferred_still_present = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM sync_deferred_mutations WHERE sync_id = ?1)",
                        [&sync_id],
                        |row| row.get::<_, bool>(0),
                    )?;
                    if !deferred_still_present {
                        tx.commit()?;
                        result.succeeded += 1;
                        continue;
                    }
                    let retry = retry_count + 1;
                    let dead = retry >= DEFERRED_DEAD_THRESHOLD;
                    record_deferred_attempt_tx(
                        &tx,
                        &sync_id,
                        retry,
                        dead,
                        "referenced observations are still missing",
                    )?;
                    tx.commit()?;
                    if dead {
                        result.dead += 1;
                    } else {
                        result.failed += 1;
                    }
                }
                Err(error) => {
                    drop(tx);
                    let tx = self.write_transaction()?;
                    record_deferred_attempt_tx(
                        &tx,
                        &sync_id,
                        retry_count + 1,
                        true,
                        &error.to_string(),
                    )?;
                    tx.commit()?;
                    result.dead += 1;
                }
            }
        }
        Ok(result)
    }

    pub fn deferred_sync_counts(&self) -> Result<(i64, i64), StoreError> {
        let deferred = self.connection.query_row(
            "SELECT COUNT(*) FROM sync_deferred_mutations WHERE apply_status = 'deferred'",
            [],
            |row| row.get(0),
        )?;
        let dead = self.connection.query_row(
            "SELECT COUNT(*) FROM sync_deferred_mutations WHERE apply_status = 'dead'",
            [],
            |row| row.get(0),
        )?;
        Ok((deferred, dead))
    }
}
