-- Schema version 1: tables.
--
-- Runs first, before the legacy column adds and table rebuilds that
-- `adopt_to_baseline` performs by inspection. Safe to re-run.
--
-- Released migrations are never edited. A new change is a new numbered file.

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    directory TEXT NOT NULL,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    ended_at TEXT,
    summary TEXT
);

CREATE TABLE IF NOT EXISTS observations (
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
    pinned BOOLEAN NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    deleted_at TEXT,
    review_after TEXT,
    expires_at TEXT,
    -- Inherited from the upstream schema and never written. Leteo does not
    -- embed anything: retrieval is FTS5 with weighted BM25, and the semantic
    -- half is a language model judging a pair through an agent CLI, which needs
    -- no vector index and keeps the store one file with no server. A real store
    -- of three and a half thousand memories holds no embedding at all. They
    -- stay because an adopted database has them, not because anything reads
    -- them -- and because whoever does want vectors one day will find the
    -- storage already shaped for it.
    embedding BLOB,
    embedding_model TEXT,
    embedding_created_at TEXT,
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

-- `porter unicode61` rather than the default tokenizer, so a word and its
-- plural are the same word.
--
-- Without the stemmer `evaluation` and `evaluations` are two unrelated terms
-- and a search for one cannot find the other. Measured on a real store of 3,408
-- memories: MRR moved +1.5 points on long questions and +1.6 on short ones,
-- both intervals clear of zero over four hundred questions.
--
-- It matters in Spanish too, which is not obvious for an English stemmer:
-- Porter's first step strips a trailing `s`, so `memoria`/`memorias`,
-- `sesion`/`sesiones` and `busqueda`/`busquedas` all collapse the same way.
--
-- It costs nothing: fewer distinct terms means a *smaller* index — 5.73 MB to
-- 5.14 MB on that store — and searches came out marginally faster.
CREATE VIRTUAL TABLE IF NOT EXISTS observations_fts USING fts5(
    title, content, tool_name, type, project, topic_key,
    content='observations', content_rowid='id',
    tokenize = 'porter unicode61'
);

CREATE TABLE IF NOT EXISTS prompts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    sync_id TEXT,
    session_id TEXT NOT NULL,
    content TEXT NOT NULL,
    project TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (session_id) REFERENCES sessions(id)
);

CREATE TABLE IF NOT EXISTS prompt_deletions (
    sync_id TEXT PRIMARY KEY,
    session_id TEXT,
    project TEXT,
    deleted_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE VIRTUAL TABLE IF NOT EXISTS prompts_fts USING fts5(
    content, project, content='prompts', content_rowid='id',
    tokenize = 'porter unicode61'
);

CREATE TABLE IF NOT EXISTS sync_chunks (
    target_key TEXT NOT NULL DEFAULT 'local',
    chunk_id TEXT NOT NULL,
    imported_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (target_key, chunk_id)
);

CREATE TABLE IF NOT EXISTS sync_state (
    target_key TEXT PRIMARY KEY,
    lifecycle TEXT NOT NULL DEFAULT 'idle',
    last_enqueued_seq INTEGER NOT NULL DEFAULT 0,
    last_acked_seq INTEGER NOT NULL DEFAULT 0,
    last_pulled_seq INTEGER NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    backoff_until TEXT,
    lease_owner TEXT,
    lease_until TEXT,
    reason_code TEXT,
    reason_message TEXT,
    last_error TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS sync_mutations (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    target_key TEXT NOT NULL,
    entity TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    op TEXT NOT NULL,
    payload TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'local',
    project TEXT NOT NULL DEFAULT '',
    occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
    acked_at TEXT,
    FOREIGN KEY (target_key) REFERENCES sync_state(target_key)
);

CREATE TABLE IF NOT EXISTS memory_relations (
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

CREATE TABLE IF NOT EXISTS sync_enrolled_projects (
    project TEXT PRIMARY KEY,
    enrolled_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS sync_deferred_mutations (
    sync_id TEXT PRIMARY KEY,
    entity TEXT NOT NULL,
    payload TEXT NOT NULL,
    apply_status TEXT NOT NULL DEFAULT 'deferred',
    retry_count INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    last_attempted_at TEXT,
    first_seen_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS sync_upgrade_state (
    project TEXT PRIMARY KEY,
    stage TEXT NOT NULL DEFAULT 'planned',
    repair_class TEXT NOT NULL DEFAULT 'none',
    snapshot_json TEXT NOT NULL DEFAULT '{}',
    last_error_code TEXT,
    last_error_message TEXT,
    findings_json TEXT,
    applied_actions TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
