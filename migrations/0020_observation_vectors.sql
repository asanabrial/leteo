-- Vectors: the embedding the semantic search stage compares a question with.
--
-- A table of their own and not the `embedding*` columns on `observations`,
-- which stay reserved and written by nothing. Three reasons, each measured or
-- read off the code rather than assumed:
--
--   * `obs_fts_update` and `obs_exact_update` are bare `AFTER UPDATE ON
--     observations` with no `UPDATE OF` list, so writing a vector into the row
--     re-indexes its text in both full-text indexes. Backfilling 5,336 real
--     memories into the column took 1.6 s; into a separate table, 0.08 to 0.21 s.
--   * Narrowing those triggers to `UPDATE OF title, content, ...` would also
--     change the definition `doctor --repair` restores from, for a table that
--     does not need it.
--   * Adoption copies the `embedding*` columns of an Engram database verbatim,
--     so a vector computed by some other model could arrive there. Nothing in
--     this table can be put there by a foreign store.
--
-- One row per memory, keyed by its local `id`. Vectors are derived data, local
-- to the machine that computed them: they are not replicated, not exported,
-- and not counted by `doctor`. A second machine embeds its own copy when it
-- first needs to, which is what makes the table safe to lose.
--
-- `ON DELETE CASCADE` because there are three hard-delete paths (a memory, a
-- session, a project) and a vector outliving its memory is space nobody reads.
-- It is the one rule the database can keep for all three instead of each path
-- remembering it. Foreign keys are on for every connection.
--
-- `model` names what computed the vector. A build that accepts a different model
-- finds every row stale and embeds it again; vectors from two models are not
-- comparable, and nothing is migrated in place.
--
-- `source_key` is what the vector was computed from, in a form SQL can compare
-- against the live row without reading its text: the content hash the row
-- already carries, and the title, which that hash does not cover. A row whose
-- key no longer matches is stale and is embedded again the next time the stage
-- fires. There is no hook on the write paths for this to keep in step with.
--
-- `vector` is the 256 components as little-endian f32, L2-normalised, so a
-- cosine is a dot product. 1,024 bytes a memory. A text the model has no token
-- for has no mean, and is stored with a zero-length `vector` rather than not at
-- all: the row says it was embedded, so it is not found stale on every question,
-- and the scan skips any vector that is not the model's width.

CREATE TABLE IF NOT EXISTS observation_vectors (
    observation_id INTEGER PRIMARY KEY REFERENCES observations(id) ON DELETE CASCADE,
    model TEXT NOT NULL,
    source_key TEXT NOT NULL,
    vector BLOB NOT NULL
);
