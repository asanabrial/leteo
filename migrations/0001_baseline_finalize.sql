-- Schema version 1: backfills, indexes and full-text triggers.
--
-- Runs last, after every column exists: the triggers reference columns that a
-- legacy database only gains during adoption, so this cannot precede it.

-- Backfills for columns added to rows that predate them.
UPDATE sync_mutations
SET project = COALESCE(json_extract(payload, '$.project'), '')
WHERE project = '' AND payload != '';

UPDATE sync_mutations
SET project = COALESCE((
    SELECT sessions.project
    FROM sessions
    WHERE sessions.id = json_extract(sync_mutations.payload, '$.session_id')
), '')
WHERE project = ''
  AND payload != ''
  AND ifnull(json_extract(payload, '$.session_id'), '') != '';

UPDATE observations SET scope = 'project' WHERE scope IS NULL OR scope = '';
UPDATE observations SET topic_key = NULL WHERE topic_key = '';
UPDATE observations SET revision_count = 1
WHERE revision_count IS NULL OR revision_count < 1;
UPDATE observations SET duplicate_count = 1
WHERE duplicate_count IS NULL OR duplicate_count < 1;
UPDATE observations SET updated_at = created_at
WHERE updated_at IS NULL OR updated_at = '';
UPDATE observations
SET sync_id = 'obs-' || lower(hex(randomblob(16)))
WHERE sync_id IS NULL OR sync_id = '';

UPDATE prompts SET project = '' WHERE project IS NULL;
UPDATE prompt_deletions SET project = '' WHERE project IS NULL;
UPDATE prompts
SET sync_id = 'prompt-' || lower(hex(randomblob(16)))
WHERE sync_id IS NULL OR sync_id = '';

INSERT OR IGNORE INTO sync_state (target_key, lifecycle, updated_at)
VALUES ('cloud', 'idle', datetime('now'));

-- Indexes.
CREATE INDEX IF NOT EXISTS idx_obs_session ON observations(session_id);
CREATE INDEX IF NOT EXISTS idx_obs_type ON observations(type);
CREATE INDEX IF NOT EXISTS idx_obs_project ON observations(project);
CREATE INDEX IF NOT EXISTS idx_obs_created ON observations(created_at DESC);
-- The order every listing asks for, expression and tie-break included.
--
-- `idx_obs_created` cannot answer it: the queries sort by
-- `datetime(created_at) DESC, id DESC`, and an index on the raw column covers
-- neither the function nor the second key. So SQLite read every undeleted row
-- into a temporary b-tree and called `datetime` once per row to return the
-- first four hundred — 37 ms on a store of 3,706 memories, against 1.4 ms with
-- this. The dashboard's unfiltered page is the one that pays it; a page
-- narrowed to a project is answered by `idx_obs_project` and never noticed.
--
-- The function stays in the queries rather than being dropped for speed.
-- Ordering by the raw text is only the same order while every row is written
-- in one format, and `timestamp::parse` deliberately accepts a second one from
-- adoption and from the cloud. Ordering by `id` alone is not the same order at
-- all: adoption inserts old memories under new ids, and 94 of the newest 400
-- change places.
CREATE INDEX IF NOT EXISTS idx_obs_created_order
ON observations(datetime(created_at) DESC, id DESC);
CREATE INDEX IF NOT EXISTS idx_obs_scope ON observations(scope);
CREATE INDEX IF NOT EXISTS idx_obs_sync_id ON observations(sync_id);
CREATE INDEX IF NOT EXISTS idx_obs_topic
ON observations(topic_key, project, scope, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_obs_deleted ON observations(deleted_at);
CREATE INDEX IF NOT EXISTS idx_obs_dedupe
ON observations(normalized_hash, project, scope, type, title, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_prompts_session ON prompts(session_id);
CREATE INDEX IF NOT EXISTS idx_prompts_project ON prompts(project);
CREATE INDEX IF NOT EXISTS idx_prompts_created ON prompts(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_prompts_sync_id ON prompts(sync_id);
CREATE INDEX IF NOT EXISTS idx_prompt_deletions_project
ON prompt_deletions(project, deleted_at DESC);

CREATE INDEX IF NOT EXISTS idx_sync_mutations_target_seq
ON sync_mutations(target_key, seq);
CREATE INDEX IF NOT EXISTS idx_sync_mutations_pending
ON sync_mutations(target_key, acked_at, seq);
CREATE INDEX IF NOT EXISTS idx_sync_mutations_project ON sync_mutations(project);
CREATE INDEX IF NOT EXISTS idx_sync_mutations_lookup
ON sync_mutations(target_key, entity, entity_key, source);

CREATE INDEX IF NOT EXISTS idx_memrel_source
ON memory_relations(source_id, judgment_status);
CREATE INDEX IF NOT EXISTS idx_memrel_target
ON memory_relations(target_id, judgment_status);
CREATE INDEX IF NOT EXISTS idx_memrel_supersede
ON memory_relations(superseded_by_relation_id);
CREATE INDEX IF NOT EXISTS idx_memrel_status_created
ON memory_relations(judgment_status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_sad_status_seen
ON sync_deferred_mutations(apply_status, first_seen_at);
CREATE INDEX IF NOT EXISTS idx_sync_upgrade_state_stage
ON sync_upgrade_state(stage);

-- Full-text triggers. Dropped and recreated, so this file must run as a unit.
DROP TRIGGER IF EXISTS obs_fts_insert;
DROP TRIGGER IF EXISTS obs_fts_delete;
DROP TRIGGER IF EXISTS obs_fts_update;
CREATE TRIGGER obs_fts_insert AFTER INSERT ON observations BEGIN
    INSERT INTO observations_fts(rowid, title, content, tool_name, type, project, topic_key)
    VALUES (new.id, new.title, new.content, new.tool_name, new.type, new.project, new.topic_key);
END;
CREATE TRIGGER obs_fts_delete AFTER DELETE ON observations BEGIN
    INSERT INTO observations_fts(observations_fts, rowid, title, content, tool_name, type, project, topic_key)
    VALUES ('delete', old.id, old.title, old.content, old.tool_name, old.type, old.project, old.topic_key);
END;
CREATE TRIGGER obs_fts_update AFTER UPDATE ON observations BEGIN
    INSERT INTO observations_fts(observations_fts, rowid, title, content, tool_name, type, project, topic_key)
    VALUES ('delete', old.id, old.title, old.content, old.tool_name, old.type, old.project, old.topic_key);
    INSERT INTO observations_fts(rowid, title, content, tool_name, type, project, topic_key)
    VALUES (new.id, new.title, new.content, new.tool_name, new.type, new.project, new.topic_key);
END;

DROP TRIGGER IF EXISTS prompt_fts_insert;
DROP TRIGGER IF EXISTS prompt_fts_delete;
DROP TRIGGER IF EXISTS prompt_fts_update;
CREATE TRIGGER prompt_fts_insert AFTER INSERT ON prompts BEGIN
    INSERT INTO prompts_fts(rowid, content, project) VALUES (new.id, new.content, new.project);
END;
CREATE TRIGGER prompt_fts_delete AFTER DELETE ON prompts BEGIN
    INSERT INTO prompts_fts(prompts_fts, rowid, content, project)
    VALUES ('delete', old.id, old.content, old.project);
END;
CREATE TRIGGER prompt_fts_update AFTER UPDATE ON prompts BEGIN
    INSERT INTO prompts_fts(prompts_fts, rowid, content, project)
    VALUES ('delete', old.id, old.content, old.project);
    INSERT INTO prompts_fts(rowid, content, project) VALUES (new.id, new.content, new.project);
END;
