-- Hard-delete tombstones for memories.
--
-- A hard delete removes the observation row and its version rows, and queues a
-- delete for every peer. The journal queues a memory's versions BEFORE the
-- memory's own upsert, so a peer cannot tell "a version whose memory has not
-- arrived yet" from "a version whose memory was deleted": a guard that skipped
-- every version with no memory present would drop legitimate ones. A version
-- that arrived after the delete therefore re-created a row holding the very
-- text the delete was meant to remove.
--
-- This table is what tells the two apart: one row per `sync_id` that a hard
-- delete removed, written in the same transaction as the delete, and consulted
-- when a replicated version is applied. It is the same pattern `prompt_deletions`
-- gives prompts, without the session and project columns, because nothing reads
-- them here: the key is the whole question.
--
-- Only the version apply path reads it. A memory's own upsert is not refused by
-- it, and neither is an import, which restores a store somebody wrote down on
-- purpose.
--
-- No full-text index carries a column of this table, so the migration needs no
-- rebuild.

CREATE TABLE IF NOT EXISTS observation_deletions (
    sync_id TEXT PRIMARY KEY,
    deleted_at TEXT NOT NULL DEFAULT (datetime('now'))
);
