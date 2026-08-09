//! Fixtures every area shares, and the tests that belong to no one area.

use super::*;
use tempfile::TempDir;

mod diagnostics;
mod observations;
mod projects;
mod prompts;
mod relations;
mod replication;
mod schema;
mod search;
mod sessions;

/// A store whose default test project replicates.
///
/// Enrolled because most of what is asserted here is what the journal
/// records, and nothing is journalled for a project nobody replicates.
/// A store that is not enrolled is exercised by
/// `nothing_is_journalled_until_a_project_is_enrolled`.
fn store() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("leteo.db"));
    let mut store = Store::open(config).unwrap();
    store.enroll_project("leteo").unwrap();
    (temp, store)
}

/// A store with nothing enrolled, for asserting what enrolment changes.
fn bare_store() -> (TempDir, Store) {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("leteo.db"));
    (temp, Store::open(config).unwrap())
}

fn observation(session: &str, title: &str, content: &str) -> AddObservation {
    AddObservation {
        session_id: session.to_owned(),
        kind: "discovery".to_owned(),
        title: title.to_owned(),
        content: content.to_owned(),
        tool_name: None,
        project: Some("Leteo".to_owned()),
        scope: "project".to_owned(),
        topic_key: None,
        prompt_sync_id: None,
    }
}

/// Extracts the name each column of a select list is exposed under.
fn exposed_names(columns: &str) -> Vec<String> {
    let mut depth = 0_i32;
    let mut current = String::new();
    let mut parts = Vec::new();
    for character in columns.chars() {
        match character {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(character);
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|part| {
            let part = part.trim();
            // An alias wins; otherwise the name is whatever follows the
            // last table qualifier.
            part.rsplit_once(" AS ").map_or_else(
                || {
                    part.rsplit_once('.')
                        .map_or(part, |(_, name)| name)
                        .to_owned()
                },
                |(_, alias)| alias.trim().to_owned(),
            )
        })
        .collect()
}

/// The exact shape each table is expected to have after migration.
///
/// Without an ORM nothing checks a query against the schema at build time,
/// and the adoption path actively hides mistakes: rename a column in a
/// migration and `add_column_if_missing` quietly adds the old name back,
/// leaving both. Everything still passes, and the database carries a dead
/// column nobody notices. Asserting the whole set — not just that the
/// columns we read exist — is what catches that.
const EXPECTED_COLUMNS: &[(&str, &[&str])] = &[
    (
        "observations",
        &[
            "id",
            "sync_id",
            "session_id",
            "type",
            "title",
            "content",
            "tool_name",
            "project",
            "scope",
            "topic_key",
            "normalized_hash",
            "revision_count",
            "duplicate_count",
            "last_seen_at",
            "pinned",
            "created_at",
            "updated_at",
            "deleted_at",
            "review_after",
            "expires_at",
            "embedding",
            "embedding_model",
            "embedding_created_at",
            "prompt_sync_id",
        ],
    ),
    (
        "sessions",
        &[
            "id",
            "project",
            "directory",
            "started_at",
            "ended_at",
            "summary",
        ],
    ),
    (
        "prompts",
        &[
            "id",
            "sync_id",
            "session_id",
            "content",
            "project",
            "created_at",
        ],
    ),
    (
        "sync_mutations",
        &[
            "seq",
            "target_key",
            "entity",
            "entity_key",
            "op",
            "payload",
            "source",
            "project",
            "occurred_at",
            "acked_at",
        ],
    ),
];

fn legacy_database(schema: &str) -> (TempDir, StoreConfig) {
    let temp = TempDir::new().unwrap();
    let config = StoreConfig::new(temp.path().join("engram.db"));
    let connection = Connection::open(&config.database_path).unwrap();
    connection.execute_batch(schema).unwrap();
    drop(connection);
    (temp, config)
}

fn observation_rows(connection: &Connection) -> Vec<(i64, String, String)> {
    let mut statement = connection
        .prepare("SELECT id, sync_id, content FROM observations ORDER BY rowid")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

const EARLY_ENGRAM_SCHEMA: &str = r#"
    CREATE TABLE sessions (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        directory TEXT NOT NULL,
        started_at TEXT NOT NULL DEFAULT (datetime('now')),
        ended_at TEXT,
        summary TEXT
    );

    CREATE TABLE observations (
        id INT,
        session_id TEXT,
        type TEXT,
        title TEXT,
        content TEXT,
        tool_name TEXT,
        project TEXT,
        created_at TEXT
    );

    CREATE VIRTUAL TABLE observations_fts USING fts5(
        title, content, tool_name, type, project,
        content='observations', content_rowid='id'
    );

    CREATE TABLE prompts (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id TEXT NOT NULL,
        content TEXT NOT NULL,
        project TEXT,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE VIRTUAL TABLE prompts_fts USING fts5(
        content, project, content='prompts', content_rowid='id'
    );

    CREATE TRIGGER prompt_fts_insert AFTER INSERT ON prompts BEGIN
        INSERT INTO prompts_fts(rowid, content, project)
        VALUES (new.id, new.content, new.project);

    END;

    CREATE TRIGGER prompt_fts_delete AFTER DELETE ON prompts BEGIN
        INSERT INTO prompts_fts(prompts_fts, rowid, content, project)
        VALUES ('delete', old.id, old.content, old.project);

    END;

    CREATE TRIGGER prompt_fts_update AFTER UPDATE ON prompts BEGIN
        INSERT INTO prompts_fts(prompts_fts, rowid, content, project)
        VALUES ('delete', old.id, old.content, old.project);

        INSERT INTO prompts_fts(rowid, content, project)
        VALUES (new.id, new.content, new.project);

    END;

    CREATE TABLE sync_chunks (
        chunk_id TEXT PRIMARY KEY,
        imported_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE sync_state (
        target_key TEXT PRIMARY KEY,
        lifecycle TEXT NOT NULL DEFAULT 'idle',
        last_enqueued_seq INTEGER NOT NULL DEFAULT 0,
        last_acked_seq INTEGER NOT NULL DEFAULT 0,
        last_pulled_seq INTEGER NOT NULL DEFAULT 0,
        consecutive_failures INTEGER NOT NULL DEFAULT 0,
        backoff_until TEXT,
        lease_owner TEXT,
        lease_until TEXT,
        last_error TEXT,
        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE sync_mutations (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        target_key TEXT NOT NULL,
        entity TEXT NOT NULL,
        entity_key TEXT NOT NULL,
        op TEXT NOT NULL,
        payload TEXT NOT NULL,
        source TEXT NOT NULL DEFAULT 'local',
        occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
        acked_at TEXT,
        FOREIGN KEY (target_key) REFERENCES sync_state(target_key)
    );

    INSERT INTO sessions (id, project, directory)
    VALUES ('legacy-session', 'engram', '/tmp/engram');

    INSERT INTO observations (id, session_id, type, title, content, project, created_at)
    VALUES
        (NULL, 'legacy-session', 'bugfix', 'Legacy null', 'legacy null content', 'engram', '2024-01-01 00:00:00'),
        (7, 'legacy-session', 'bugfix', 'Legacy fixed', 'legacy fixed content', 'engram', '2024-01-02 00:00:00'),
        (7, 'legacy-session', 'bugfix', 'Legacy duplicate', 'legacy duplicate content', 'engram', '2024-01-03 00:00:00');

    INSERT INTO prompts (session_id, content, project)
    VALUES ('legacy-session', 'legacy prompt', NULL);

    INSERT INTO sync_chunks (chunk_id, imported_at)
    VALUES ('chunk-legacy', '2024-02-01 00:00:00');

    INSERT INTO sync_state (target_key) VALUES ('cloud');

    INSERT INTO sync_mutations (target_key, entity, entity_key, op, payload)
    VALUES ('cloud', 'observation', 'obs-old', 'upsert',
            '{"session_id":"legacy-session"}');

    -- Enrolled, because that is what makes the journal row above mean
    -- something. Adoption fills in its `project` so the queue can be filtered
    -- by one, and a project nobody replicates has no queue to filter: the
    -- upgrade drops those, since enrolling a project throws its pending
    -- mutations away and backfills from scratch anyway.
    CREATE TABLE IF NOT EXISTS sync_enrolled_projects (
        project TEXT PRIMARY KEY,
        enrolled_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    INSERT INTO sync_enrolled_projects (project) VALUES ('engram');

    "#;

const PRE_CONFLICT_SCHEMA_WITH_OLD_FTS: &str = r#"
    CREATE TABLE sessions (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        directory TEXT NOT NULL,
        started_at TEXT NOT NULL DEFAULT (datetime('now')),
        ended_at TEXT,
        summary TEXT
    );

    CREATE TABLE observations (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        sync_id TEXT,
        session_id TEXT NOT NULL,
        type TEXT NOT NULL,
        title TEXT NOT NULL,
        content TEXT NOT NULL,
        tool_name TEXT,
        project TEXT,
        scope TEXT NOT NULL DEFAULT 'project',
        topic_key TEXT,
        normalized_hash TEXT,
        revision_count INTEGER NOT NULL DEFAULT 1,
        duplicate_count INTEGER NOT NULL DEFAULT 1,
        last_seen_at TEXT,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        updated_at TEXT NOT NULL DEFAULT '',
        deleted_at TEXT,
        FOREIGN KEY (session_id) REFERENCES sessions(id)
    );

    CREATE VIRTUAL TABLE observations_fts USING fts5(
        title, content, tool_name, type, project,
        content='observations', content_rowid='id'
    );

    CREATE TRIGGER obs_fts_insert AFTER INSERT ON observations BEGIN
        INSERT INTO observations_fts(rowid, title, content, tool_name, type, project)
        VALUES (new.id, new.title, new.content, new.tool_name, new.type, new.project);

    END;

    CREATE TRIGGER obs_fts_delete AFTER DELETE ON observations BEGIN
        INSERT INTO observations_fts(observations_fts, rowid, title, content, tool_name, type, project)
        VALUES ('delete', old.id, old.title, old.content, old.tool_name, old.type, old.project);

    END;

    CREATE TRIGGER obs_fts_update AFTER UPDATE ON observations BEGIN
        INSERT INTO observations_fts(observations_fts, rowid, title, content, tool_name, type, project)
        VALUES ('delete', old.id, old.title, old.content, old.tool_name, old.type, old.project);

        INSERT INTO observations_fts(rowid, title, content, tool_name, type, project)
        VALUES (new.id, new.title, new.content, new.tool_name, new.type, new.project);

    END;

    INSERT INTO sessions (id, project, directory)
    VALUES ('pre-conflict', 'engram', '/tmp/engram');

    INSERT INTO observations (
        sync_id, session_id, type, title, content, project, scope, topic_key,
        revision_count, duplicate_count, created_at, updated_at
    ) VALUES (
        'obs-pre-conflict', 'pre-conflict', 'bugfix', 'Fixed tokenizer',
        'Normalized tokenizer panic on edge case', 'engram', '', '', 0, 0,
        '2024-03-01 10:00:00', ''
    );

    "#;

const POST_CONFLICT_SCHEMA: &str = r#"
    CREATE TABLE sessions (
        id TEXT PRIMARY KEY,
        project TEXT NOT NULL,
        directory TEXT NOT NULL,
        started_at TEXT NOT NULL DEFAULT (datetime('now')),
        ended_at TEXT,
        summary TEXT
    );

    CREATE TABLE observations (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        sync_id TEXT,
        session_id TEXT NOT NULL,
        type TEXT NOT NULL,
        title TEXT NOT NULL,
        content TEXT NOT NULL,
        tool_name TEXT,
        project TEXT,
        scope TEXT NOT NULL DEFAULT 'project',
        topic_key TEXT,
        normalized_hash TEXT,
        revision_count INTEGER NOT NULL DEFAULT 1,
        duplicate_count INTEGER NOT NULL DEFAULT 1,
        last_seen_at TEXT,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        updated_at TEXT NOT NULL DEFAULT (datetime('now')),
        deleted_at TEXT,
        review_after TEXT,
        expires_at TEXT,
        embedding BLOB,
        embedding_model TEXT,
        embedding_created_at TEXT
    );

    CREATE VIRTUAL TABLE observations_fts USING fts5(
        title, content, tool_name, type, project, topic_key,
        content='observations', content_rowid='id'
    );

    CREATE TRIGGER obs_fts_insert AFTER INSERT ON observations BEGIN
        INSERT INTO observations_fts(rowid, title, content, tool_name, type, project, topic_key)
        VALUES (new.id, new.title, new.content, new.tool_name, new.type, new.project, new.topic_key);

    END;

    CREATE TABLE memory_relations (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        sync_id TEXT NOT NULL UNIQUE,
        source_id TEXT,
        target_id TEXT,
        relation TEXT NOT NULL DEFAULT 'pending',
        reason TEXT,
        evidence TEXT,
        confidence REAL,
        judgment_status TEXT NOT NULL DEFAULT 'pending',
        marked_by_actor TEXT,
        marked_by_kind TEXT,
        marked_by_model TEXT,
        session_id TEXT,
        superseded_at TEXT,
        superseded_by_relation_id INTEGER REFERENCES memory_relations(id) ON DELETE SET NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now')),
        updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    INSERT INTO sessions (id, project, directory)
    VALUES ('post-conflict', 'engram', '/tmp/engram');

    INSERT INTO observations (sync_id, session_id, type, title, content, project)
    VALUES
        ('obs-source', 'post-conflict', 'decision', 'Use Redis', 'Redis caching decision', 'engram'),
        ('obs-target', 'post-conflict', 'decision', 'Use Memcached', 'Alternative caching decision', 'engram');

    INSERT INTO memory_relations (
        sync_id, source_id, target_id, relation, judgment_status
    ) VALUES (
        'rel-legacy', 'obs-source', 'obs-target', 'conflicts_with', 'judged'
    );

    "#;
