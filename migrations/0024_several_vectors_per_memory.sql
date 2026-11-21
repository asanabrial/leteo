-- Several vectors per memory: one for each window of its text.
--
-- Migration 20 kept one vector per memory, the mean of its first 128 tokens. On
-- a copy of a real store the median body was 548 tokens and 98% of bodies were
-- longer than 128, so the stage never read most of what a memory says: a blind
-- question about a fact in the second half of a body found its memory in the
-- top ten 50.7% of the time in the memory's language and 3.6% in another. Read
-- as windows of 128 tokens, each headed by the title and scored by the best of
-- them, the same questions found it 62.1% and 25.0% of the time.
-- `semantic::windows` cuts a memory into those windows; this table keeps one row
-- per window.
--
-- The rows of migration 20 are dropped rather than converted. Each is the mean
-- of a text no window reproduces exactly, so every one would be stale and
-- embedded again anyway -- the table holds only what can be recomputed, which
-- is what migration 20 made it for -- and the backfill rebuilds them the way it
-- builds them for a store that never had any. Until it has, a memory is found by
-- its words only, which `leteo doctor` reports and `leteo doctor --repair` mends.
--
-- `ordinal` is the window's place in the memory, from 0. Every memory that has
-- been embedded has a row 0, even one with no token the model knows, so the
-- questions that count memories -- which are stale, how many are covered -- read
-- row 0 alone and count each memory once. Every row of one memory is written in
-- one transaction, after deleting the memory's rows, so a memory edited shorter
-- keeps no window of its longer text and all its rows carry one `model`,
-- `windowing` and `source_key`.
--
-- `windowing` names how the memory was cut (`semantic::WINDOWING`), beside
-- `model`, which names the weights. Either one changing makes every row stale.
--
-- `observation_id`, `model`, `source_key` and `vector` mean what migration 20
-- says they mean, and `ON DELETE CASCADE` is kept for the reason it gives: three
-- hard-delete paths, and one rule the database can keep for all of them. 1,024
-- bytes a row, and the measured store averaged 6.6 rows a memory.

DROP TABLE IF EXISTS observation_vectors;

CREATE TABLE observation_vectors (
    observation_id INTEGER NOT NULL REFERENCES observations(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    model TEXT NOT NULL,
    windowing TEXT NOT NULL,
    source_key TEXT NOT NULL,
    vector BLOB NOT NULL,
    PRIMARY KEY (observation_id, ordinal)
);
