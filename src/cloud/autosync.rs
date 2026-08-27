use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    memory::model::SyncMutation,
    store::{Store, StoreError},
};

use super::{
    MAX_MUTATION_BATCH_SIZE,
    cloudstore::MutationEntry,
    remote::{HttpStatusError, RemoteClient, RemoteError},
};

#[derive(Debug, Clone)]
pub struct AutosyncConfig {
    pub target_key: String,
    pub lease_owner: String,
    pub lease_ttl: Duration,
    pub poll_interval: Duration,
    pub push_batch_size: usize,
    pub pull_batch_size: usize,
    pub base_backoff: Duration,
    pub max_backoff: Duration,
    pub allowed_projects: Vec<String>,
    pub created_by: String,
}

impl Default for AutosyncConfig {
    fn default() -> Self {
        Self {
            target_key: "cloud".to_owned(),
            lease_owner: format!(
                "autosync-{}-{}",
                std::process::id(),
                Utc::now().timestamp_millis()
            ),
            lease_ttl: Duration::from_secs(60),
            poll_interval: Duration::from_secs(30),
            push_batch_size: MAX_MUTATION_BATCH_SIZE,
            pull_batch_size: MAX_MUTATION_BATCH_SIZE,
            base_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(5 * 60),
            allowed_projects: Vec::new(),
            created_by: crate::sync::created_by(),
        }
    }
}

impl AutosyncConfig {
    pub fn validate(&self) -> Result<(), AutosyncError> {
        if self.target_key.trim().is_empty() || self.lease_owner.trim().is_empty() {
            return Err(AutosyncError::InvalidConfig(
                "target key and lease owner are required".to_owned(),
            ));
        }
        if self.allowed_projects.is_empty() {
            return Err(AutosyncError::InvalidConfig(
                "at least one allowed project or wildcard is required".to_owned(),
            ));
        }
        if self.push_batch_size == 0
            || self.push_batch_size > MAX_MUTATION_BATCH_SIZE
            || self.pull_batch_size == 0
            || self.pull_batch_size > MAX_MUTATION_BATCH_SIZE
            || self.lease_ttl.is_zero()
            || self.poll_interval.is_zero()
            || self.base_backoff.is_zero()
            || self.max_backoff < self.base_backoff
        {
            return Err(AutosyncError::InvalidConfig(
                "autosync limits and durations are invalid".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutosyncStatus {
    pub phase: String,
    pub consecutive_failures: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backoff_until: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    pub deferred_count: i64,
    pub dead_count: i64,
}

pub struct Autosync<'a> {
    store: &'a mut Store,
    remote: RemoteClient,
    config: AutosyncConfig,
    status: AutosyncStatus,
}

impl<'a> Autosync<'a> {
    pub fn new(
        store: &'a mut Store,
        remote: RemoteClient,
        config: AutosyncConfig,
    ) -> Result<Self, AutosyncError> {
        config.validate()?;
        Ok(Self {
            store,
            remote,
            config,
            status: AutosyncStatus {
                phase: "idle".to_owned(),
                ..AutosyncStatus::default()
            },
        })
    }

    pub fn status(&self) -> &AutosyncStatus {
        &self.status
    }

    pub async fn run_cycle(&mut self) -> Result<bool, AutosyncError> {
        let now = Utc::now();
        let state = self.store.get_sync_state(&self.config.target_key)?;
        if state
            .backoff_until
            .as_deref()
            .and_then(parse_timestamp)
            .is_some_and(|until| until > now)
        {
            self.status.phase = "backoff".to_owned();
            self.status.backoff_until = state.backoff_until;
            return Ok(false);
        }
        let acquired = self.store.acquire_sync_lease(
            &self.config.target_key,
            &self.config.lease_owner,
            self.config.lease_ttl,
            now,
        )?;
        if !acquired {
            self.status.phase = "idle".to_owned();
            return Ok(false);
        }

        let result = self.sync_once().await;
        // A lease that cannot be released expires on its own, so a transient
        // lock is worth a warning and nothing more. Returning here instead
        // would discard the cycle's outcome, and a failed sync would never
        // record its backoff — leaving a failing cloud to be retried in a tight
        // loop.
        if let Err(error) = self
            .store
            .release_sync_lease(&self.config.target_key, &self.config.lease_owner)
        {
            tracing::warn!(%error, "could not release the sync lease; it will expire on its own");
        }
        match result {
            Ok(()) => {
                self.store.mark_sync_healthy(&self.config.target_key)?;
                let (deferred_count, dead_count) = self.store.deferred_sync_counts()?;
                self.status = AutosyncStatus {
                    phase: "healthy".to_owned(),
                    last_sync_at: Some(Utc::now().to_rfc3339()),
                    deferred_count,
                    dead_count,
                    ..AutosyncStatus::default()
                };
                Ok(true)
            }
            Err(error) => {
                let failures = state.consecutive_failures.saturating_add(1);
                let standing = standing_refusal(&error);
                let backoff = match standing {
                    Some(_) => self.config.max_backoff,
                    None => {
                        compute_backoff(self.config.base_backoff, self.config.max_backoff, failures)
                    }
                };
                let until = Utc::now()
                    + chrono::Duration::from_std(backoff).map_err(|_| {
                        AutosyncError::InvalidConfig("backoff duration is too large".to_owned())
                    })?;
                self.store
                    .mark_sync_failure(&self.config.target_key, &error.to_string(), until)?;
                self.status.phase = match standing {
                    Some(status) if status.is_auth_failure() => "unauthorized",
                    Some(_) => "refused",
                    None => "backoff",
                }
                .to_owned();
                self.status.consecutive_failures = u32::try_from(failures).unwrap_or(u32::MAX);
                self.status.last_error = Some(error.to_string());
                self.status.backoff_until = Some(until.to_rfc3339());
                Err(error)
            }
        }
    }

    pub async fn run(
        &mut self,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), AutosyncError> {
        let mut interval = tokio::time::interval(self.config.poll_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                stop = shutdown_requested(&mut shutdown) => {
                    if stop {
                        break;
                    }
                    continue;
                }
            }
            if *shutdown.borrow() {
                break;
            }
            // The cycle has to lose this race, not merely be skipped before it
            // starts. A cycle pages through the cloud one request at a time,
            // each request may take as long as the client timeout, and the page
            // count is whatever the backlog happens to be — so a caller that
            // signals shutdown and joins this thread would otherwise wait
            // minutes for a cycle nobody is waiting on any more.
            //
            // Dropping the cycle's future is safe: every await inside it is an
            // HTTP call, and each mutation it already applied was committed on
            // its own. The lease it abandons expires on its own TTL.
            tokio::select! {
                stop = shutdown_requested(&mut shutdown) => {
                    if stop {
                        break;
                    }
                }
                result = self.run_cycle() => {
                    if let Err(error) = result {
                        tracing::warn!(%error, "autosync cycle failed");
                    }
                }
            }
        }
        if let Err(error) = self
            .store
            .release_sync_lease(&self.config.target_key, &self.config.lease_owner)
        {
            tracing::warn!(%error, "could not release the sync lease on shutdown");
        }
        Ok(())
    }

    async fn sync_once(&mut self) -> Result<(), AutosyncError> {
        self.push_pending().await?;
        self.pull_remote().await?;
        Ok(())
    }

    async fn push_pending(&mut self) -> Result<(), AutosyncError> {
        self.status.phase = "pushing".to_owned();
        loop {
            let pending = self.store.list_pending_sync_mutations(
                &self.config.target_key,
                &self.config.allowed_projects,
                self.config.push_batch_size,
            )?;
            if pending.is_empty() {
                return Ok(());
            }
            let local_sequences = pending
                .iter()
                .map(|mutation| mutation.seq)
                .collect::<Vec<_>>();
            let entries = pending
                .iter()
                .map(local_mutation_entry)
                .collect::<Result<Vec<_>, _>>()?;
            let response = self
                .remote
                .push_mutations(&entries, &self.config.created_by)
                .await?;
            if response.accepted_seqs.len() != local_sequences.len() {
                return Err(AutosyncError::Protocol(format!(
                    "cloud accepted {} of {} mutations; local rows were not ACKed",
                    response.accepted_seqs.len(),
                    local_sequences.len()
                )));
            }
            self.store
                .ack_sync_mutation_seqs(&self.config.target_key, &local_sequences)?;
            if pending.len() < self.config.push_batch_size {
                return Ok(());
            }
        }
    }

    async fn pull_remote(&mut self) -> Result<(), AutosyncError> {
        self.status.phase = "pulling".to_owned();
        self.store.replay_deferred_sync_mutations()?;
        let mut cursor = self
            .store
            .get_sync_state(&self.config.target_key)?
            .last_pulled_seq;
        loop {
            let response = self
                .remote
                .pull_mutations(cursor, self.config.pull_batch_size)
                .await?;
            if response.has_more && response.mutations.is_empty() {
                return Err(AutosyncError::Protocol(
                    "cloud returned has_more without mutations".to_owned(),
                ));
            }
            let cursor_before = cursor;
            // A page has to arrive in sequence order, and it is checked rather
            // than assumed.
            //
            // The cursor advances to whatever this loop last applied, and it is
            // persisted. So if a page ever came back out of order — sequence 10
            // ahead of sequence 6 — applying 10 would move the cursor past 6,
            // 6 would be skipped as already-seen on the way past, and the next
            // pull would ask for everything after 10. Nothing would ever ask
            // for 6 again. The memory it carried would be gone, permanently,
            // with no error anywhere: the sync would report success.
            //
            // Leteo's own server orders by `seq`, so this never fires against
            // it. It is not the only thing a client can be pointed at, and the
            // loop already refuses a page that is empty or that fails to
            // advance — this is the third way the same contract can be broken,
            // and the only one of the three that loses data quietly.
            let mut previous: Option<i64> = None;
            for mutation in response.mutations {
                if let Some(previous) = previous
                    && mutation.seq <= previous
                {
                    return Err(AutosyncError::Protocol(format!(
                        "cloud returned sequence {} after {previous}; a page of \
                         mutations has to be ordered or the ones behind the \
                         cursor are lost",
                        mutation.seq
                    )));
                }
                previous = Some(mutation.seq);
                if mutation.seq <= cursor {
                    continue;
                }
                let local = SyncMutation {
                    seq: mutation.seq,
                    target_key: self.config.target_key.clone(),
                    entity: mutation.entity,
                    entity_key: mutation.entity_key,
                    op: mutation.op,
                    payload: serde_json::to_string(&mutation.payload)?,
                    source: "remote".to_owned(),
                    project: mutation.project,
                    occurred_at: mutation.occurred_at,
                    acked_at: None,
                };
                self.store
                    .apply_pulled_sync_mutation(&self.config.target_key, &local)?;
                cursor = mutation.seq;
            }
            if !continue_pulling(response.has_more, cursor_before, cursor)? {
                return Ok(());
            }
        }
    }
}

async fn shutdown_requested(shutdown: &mut tokio::sync::watch::Receiver<bool>) -> bool {
    match shutdown.changed().await {
        Ok(()) => *shutdown.borrow(),
        Err(_) => true,
    }
}

fn continue_pulling(
    has_more: bool,
    cursor_before: i64,
    cursor_after: i64,
) -> Result<bool, AutosyncError> {
    if !has_more {
        return Ok(false);
    }
    if cursor_after <= cursor_before {
        return Err(AutosyncError::Protocol(format!(
            "cloud returned has_more without advancing past sequence {cursor_before}"
        )));
    }
    Ok(true)
}

#[derive(Debug, Error)]
pub enum AutosyncError {
    #[error("invalid autosync configuration: {0}")]
    InvalidConfig(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Remote(#[from] RemoteError),
    #[error("cloud sync protocol error: {0}")]
    Protocol(String),
    #[error("cloud sync payload: {0}")]
    Json(#[from] serde_json::Error),
}

fn local_mutation_entry(mutation: &SyncMutation) -> Result<MutationEntry, AutosyncError> {
    let payload = serde_json::from_str(&mutation.payload)?;
    Ok(MutationEntry {
        project: mutation.project.clone(),
        entity: mutation.entity.clone(),
        entity_key: mutation.entity_key.clone(),
        op: mutation.op.clone(),
        payload,
    })
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    crate::timestamp::parse(value).map(|timestamp| timestamp.and_utc())
}

fn standing_refusal(error: &AutosyncError) -> Option<&HttpStatusError> {
    let AutosyncError::Remote(RemoteError::Status(status)) = error else {
        return None;
    };
    (status.is_auth_failure() || status.is_policy_failure()).then_some(status)
}

fn compute_backoff(base: Duration, maximum: Duration, failures: i64) -> Duration {
    let exponent = u32::try_from(failures.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .min(31);
    base.saturating_mul(1_u32 << exponent).min(maximum)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn config_rejects_unbounded_or_unscoped_sync() {
        let mut config = AutosyncConfig::default();
        assert!(config.validate().is_err());
        config.allowed_projects = vec!["proj-a".to_owned()];
        assert!(config.validate().is_ok());
        config.push_batch_size = 101;
        assert!(config.validate().is_err());
    }

    #[test]
    fn pulling_stops_instead_of_spinning_when_the_cursor_cannot_advance() {
        assert!(continue_pulling(true, 10, 42).unwrap());
        assert!(!continue_pulling(false, 10, 42).unwrap());
        assert!(!continue_pulling(false, 10, 10).unwrap());

        let error = continue_pulling(true, 10, 10).unwrap_err();
        assert!(
            matches!(error, AutosyncError::Protocol(ref message) if message.contains("advancing")),
            "unexpected error: {error}"
        );
        assert!(continue_pulling(true, 10, 9).is_err());
    }

    async fn silent_server() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        address
    }

    async fn accepting_server() -> (std::net::SocketAddr, Arc<AtomicUsize>) {
        use axum::{Json, Router, routing::get, routing::post};

        let pushes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&pushes);
        let app = Router::new()
            .route(
                "/sync/mutations/push",
                post(async move |Json(body): Json<serde_json::Value>| {
                    counter.fetch_add(1, Ordering::Relaxed);
                    let count = body["entries"].as_array().map_or(0, Vec::len);
                    let accepted = (1..=count as i64).collect::<Vec<_>>();
                    Json(serde_json::json!({"accepted_seqs": accepted}))
                }),
            )
            .route(
                "/sync/mutations/pull",
                get(async || {
                    Json(serde_json::json!({"mutations": [], "has_more": false, "latest_seq": 0}))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (address, pushes)
    }

    #[tokio::test]
    async fn pushing_drains_every_page_and_then_stops() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut store =
            Store::open(crate::store::StoreConfig::new(temp.path().join("leteo.db"))).unwrap();
        store.enroll_project("proj-a").unwrap();
        store.create_session("s", "proj-a", "/tmp/proj-a").unwrap();
        for index in 0..25 {
            store
                .add_observation(crate::memory::model::AddObservation {
                    session_id: "s".to_owned(),
                    kind: "decision".to_owned(),
                    title: format!("observation {index}"),
                    content: format!("body {index}"),
                    tool_name: None,
                    project: Some("proj-a".to_owned()),
                    scope: "project".to_owned(),
                    topic_key: None,
                    prompt_sync_id: None,
                })
                .unwrap();
        }
        let queued = store.pending_sync_mutation_count("cloud").unwrap();
        assert!(queued >= 25, "expected a backlog to page through: {queued}");

        let (address, pushes) = accepting_server().await;
        let remote =
            RemoteClient::new(&format!("http://{address}"), "a-token-that-is-long-enough").unwrap();
        let config = AutosyncConfig {
            allowed_projects: vec!["proj-a".to_owned()],
            push_batch_size: 8,
            ..AutosyncConfig::default()
        };
        let mut autosync = Autosync::new(&mut store, remote, config).unwrap();

        tokio::time::timeout(Duration::from_secs(10), autosync.push_pending())
            .await
            .expect("push_pending terminated")
            .unwrap();

        assert!(
            pushes.load(Ordering::Relaxed) > 1,
            "the backlog should have taken more than one page"
        );
        let left = store.pending_sync_mutation_count("cloud").unwrap();
        assert_eq!(left, 0, "every pushed mutation should have been acked");
    }

    #[tokio::test]
    async fn a_shutdown_cancels_the_cycle_instead_of_waiting_it_out() {
        let temp = tempfile::TempDir::new().unwrap();
        let mut store =
            Store::open(crate::store::StoreConfig::new(temp.path().join("leteo.db"))).unwrap();

        let address = silent_server().await;
        let remote =
            RemoteClient::new(&format!("http://{address}"), "a-token-that-is-long-enough").unwrap();
        let config = AutosyncConfig {
            allowed_projects: vec!["proj-a".to_owned()],
            poll_interval: Duration::from_millis(10),
            ..AutosyncConfig::default()
        };
        let mut autosync = Autosync::new(&mut store, remote, config).unwrap();

        let (sender, receiver) = tokio::sync::watch::channel(false);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let _ = sender.send(true);
        });

        let stopped = tokio::time::timeout(Duration::from_secs(5), autosync.run(receiver)).await;
        assert!(
            stopped.is_ok(),
            "run() did not return promptly after shutdown was signalled"
        );
        stopped.unwrap().unwrap();
    }

    async fn paging_server(sequences: Vec<i64>) -> std::net::SocketAddr {
        use axum::{Json, Router, routing::get};

        let app = Router::new().route(
            "/sync/mutations/pull",
            get(async move || {
                let mutations = sequences
                    .iter()
                    .map(|seq| {
                        serde_json::json!({
                            "seq": seq,
                            "entity": "observation",
                            "entity_key": format!("obs-{seq}"),
                            "op": "upsert",
                            "payload": {
                                "sync_id": format!("obs-{seq}"),
                                "session_id": "s1",
                                "type": "decision",
                                "title": format!("memory {seq}"),
                                "content": "body",
                                "project": "proj-a",
                                "scope": "project",
                            },
                            "project": "proj-a",
                            "occurred_at": "2026-08-02T10:00:00Z",
                        })
                    })
                    .collect::<Vec<_>>();
                Json(serde_json::json!({
                    "mutations": mutations, "has_more": false, "latest_seq": 0
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        address
    }

    async fn pull_page(sequences: Vec<i64>) -> (Result<(), AutosyncError>, i64) {
        let temp = tempfile::TempDir::new().unwrap();
        let mut store =
            Store::open(crate::store::StoreConfig::new(temp.path().join("leteo.db"))).unwrap();
        store.enroll_project("proj-a").unwrap();
        store.create_session("s1", "proj-a", "/tmp/proj-a").unwrap();
        let address = paging_server(sequences).await;
        let remote =
            RemoteClient::new(&format!("http://{address}"), "a-token-that-is-long-enough").unwrap();
        let config = AutosyncConfig {
            allowed_projects: vec!["proj-a".to_owned()],
            ..AutosyncConfig::default()
        };
        let outcome = {
            let mut autosync = Autosync::new(&mut store, remote, config).unwrap();
            tokio::time::timeout(Duration::from_secs(10), autosync.pull_remote())
                .await
                .expect("pull_remote terminated")
        };
        let cursor = store.get_sync_state("cloud").unwrap().last_pulled_seq;
        (outcome, cursor)
    }

    #[tokio::test]
    async fn a_page_that_arrives_out_of_order_is_refused_rather_than_half_applied() {
        let (ordered, cursor) = pull_page(vec![1, 2, 3]).await;
        assert!(ordered.is_ok(), "{ordered:?}");
        assert_eq!(cursor, 3, "an ordered page is applied to its end");

        let (jumbled, cursor) = pull_page(vec![1, 10, 6]).await;
        let error = jumbled.expect_err("an out-of-order page has to be refused");
        assert!(
            matches!(error, AutosyncError::Protocol(_)),
            "an unordered page is a protocol violation, not a store error: {error:?}"
        );
        assert!(
            error.to_string().contains("ordered"),
            "the error has to say what the server did wrong: {error}"
        );
        assert_eq!(cursor, 10);
    }

    #[test]
    fn backoff_is_exponential_and_capped() {
        let base = Duration::from_secs(1);
        let maximum = Duration::from_secs(8);
        assert_eq!(compute_backoff(base, maximum, 1), Duration::from_secs(1));
        assert_eq!(compute_backoff(base, maximum, 3), Duration::from_secs(4));
        assert_eq!(compute_backoff(base, maximum, 20), maximum);
    }

    #[test]
    fn timestamps_accept_rfc3339_and_sqlite_values() {
        assert!(parse_timestamp("2026-07-27T12:00:00Z").is_some());
        assert!(parse_timestamp("2026-07-27 12:00:00").is_some());
        assert!(parse_timestamp("invalid").is_none());
    }
}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    fn status(code: u16) -> AutosyncError {
        AutosyncError::Remote(RemoteError::Status(HttpStatusError {
            operation: "push".to_owned(),
            status: code,
            error_class: String::new(),
            error_code: "denied".to_owned(),
            message: "no".to_owned(),
        }))
    }

    #[test]
    fn a_refusal_waits_the_longest_wait_from_the_very_first_one() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(300);

        assert_eq!(compute_backoff(base, max, 1), Duration::from_secs(1));
        for code in [401, 403, 409] {
            assert!(
                standing_refusal(&status(code)).is_some(),
                "{code} is the server refusing this caller, not failing to answer"
            );
        }
    }

    #[test]
    fn a_server_that_merely_broke_still_gets_the_ladder() {
        for code in [500, 502, 503, 429] {
            assert!(
                standing_refusal(&status(code)).is_none(),
                "{code} may well succeed on the next attempt"
            );
        }
        assert!(standing_refusal(&AutosyncError::Protocol("truncated".to_owned())).is_none());
        assert!(
            standing_refusal(&AutosyncError::Remote(RemoteError::InvalidResponse(
                "not json".to_owned()
            )))
            .is_none()
        );
    }

    #[test]
    fn the_phase_names_what_has_to_change_rather_than_that_leteo_is_waiting() {
        let denied = status(401);
        let unauthorized = standing_refusal(&denied).expect("401 is a refusal");
        assert!(unauthorized.is_auth_failure());

        let paused = status(403);
        let refused = standing_refusal(&paused).expect("403 is a refusal");
        assert!(!refused.is_auth_failure() && refused.is_policy_failure());
    }
}
