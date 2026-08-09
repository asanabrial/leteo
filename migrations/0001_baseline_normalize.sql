-- Bringing an adopted database's *data* to the shape the code expects.
--
-- Runs once, beside the baseline tables, and only for a database arriving
-- unstamped: a brand new file, a Leteo store from before versioning, or an
-- Engram one. On a new file every statement here matches nothing, which is why
-- it can be unconditional.
--
-- These were five numbered migrations before the first release. Collapsing them
-- into the baseline is safe because nothing has shipped, and it is *right*
-- because the split was never about time: a database converges on this shape by
-- inspection, not by replaying a history it never had. An Engram database has
-- no version to migrate from.
--
-- Verified before collapsing: a store migrated step by step through all six
-- versions and a store created fresh had identical structure — 22 tables, the
-- same columns and the same indexes on every one.

-- ## Types, folded onto the ones the documentation promises
--
UPDATE observations SET type = CASE type
    WHEN 'bug' THEN 'bugfix'
    WHEN 'fix' THEN 'bugfix'
    WHEN 'hotfix' THEN 'bugfix'
    WHEN 'incident' THEN 'bugfix'
    WHEN 'regression' THEN 'bugfix'
    WHEN 'design' THEN 'architecture'
    WHEN 'adr' THEN 'architecture'
    WHEN 'refactor' THEN 'architecture'
    WHEN 'learning' THEN 'discovery'
    WHEN 'research' THEN 'discovery'
    WHEN 'investigation' THEN 'discovery'
    WHEN 'root_cause' THEN 'discovery'
    WHEN 'root-cause' THEN 'discovery'
    WHEN 'convention' THEN 'pattern'
    WHEN 'guideline' THEN 'pattern'
    WHEN 'rule' THEN 'pattern'
    WHEN 'setup' THEN 'config'
    WHEN 'infra' THEN 'config'
    WHEN 'infrastructure' THEN 'config'
    WHEN 'ci' THEN 'config'
    WHEN 'configuration' THEN 'config'
    WHEN 'feedback' THEN 'preference'
    WHEN 'user' THEN 'preference'
    WHEN 'preferences' THEN 'preference'
    ELSE type
END
WHERE type IN (
    'bug', 'fix', 'hotfix', 'incident', 'regression',
    'design', 'adr', 'refactor',
    'learning', 'research', 'investigation', 'root_cause', 'root-cause',
    'convention', 'guideline', 'rule',
    'setup', 'infra', 'infrastructure', 'ci', 'configuration',
    'feedback', 'user', 'preferences'
);


-- ## Project names, lowercased
--
UPDATE observations SET project = lower(project)
    WHERE project IS NOT NULL AND project <> lower(project);
UPDATE prompts SET project = lower(project)
    WHERE project IS NOT NULL AND project <> lower(project);
UPDATE sessions SET project = lower(project)
    WHERE project IS NOT NULL AND project <> lower(project);
UPDATE sync_mutations SET project = lower(project)
    WHERE project IS NOT NULL AND project <> lower(project);



-- ## Project names, with repeated separators collapsed
--
UPDATE OR IGNORE observations SET project =
    replace(replace(replace(replace(lower(project), '--', '-'), '--', '-'), '--', '-'), '--', '-')
    WHERE project IS NOT NULL AND project LIKE '%--%';
UPDATE OR IGNORE observations SET project =
    replace(replace(replace(replace(lower(project), '__', '_'), '__', '_'), '__', '_'), '__', '_')
    WHERE project IS NOT NULL AND project LIKE '%\_\_%' ESCAPE '\';
UPDATE OR IGNORE observations SET project = lower(project)
    WHERE project IS NOT NULL AND project <> lower(project);

UPDATE OR IGNORE prompts SET project =
    replace(replace(replace(replace(lower(project), '--', '-'), '--', '-'), '--', '-'), '--', '-')
    WHERE project IS NOT NULL AND project LIKE '%--%';
UPDATE OR IGNORE prompts SET project =
    replace(replace(replace(replace(lower(project), '__', '_'), '__', '_'), '__', '_'), '__', '_')
    WHERE project IS NOT NULL AND project LIKE '%\_\_%' ESCAPE '\';
UPDATE OR IGNORE prompts SET project = lower(project)
    WHERE project IS NOT NULL AND project <> lower(project);

UPDATE OR IGNORE sessions SET project =
    replace(replace(replace(replace(lower(project), '--', '-'), '--', '-'), '--', '-'), '--', '-')
    WHERE project IS NOT NULL AND project LIKE '%--%';
UPDATE OR IGNORE sessions SET project =
    replace(replace(replace(replace(lower(project), '__', '_'), '__', '_'), '__', '_'), '__', '_')
    WHERE project IS NOT NULL AND project LIKE '%\_\_%' ESCAPE '\';
UPDATE OR IGNORE sessions SET project = lower(project)
    WHERE project IS NOT NULL AND project <> lower(project);

UPDATE OR IGNORE sync_mutations SET project =
    replace(replace(replace(replace(lower(project), '--', '-'), '--', '-'), '--', '-'), '--', '-')
    WHERE project IS NOT NULL AND project LIKE '%--%';
UPDATE OR IGNORE sync_mutations SET project =
    replace(replace(replace(replace(lower(project), '__', '_'), '__', '_'), '__', '_'), '__', '_')
    WHERE project IS NOT NULL AND project LIKE '%\_\_%' ESCAPE '\';
UPDATE OR IGNORE sync_mutations SET project = lower(project)
    WHERE project IS NOT NULL AND project <> lower(project);


-- ## The index, recreated with the stemmer and rebuilt once
--
-- Dropped and recreated rather than rebuilt in place, and that distinction is a
-- bug this file was written without. The baseline creates both indexes with
-- `porter unicode61`, but it does so with `IF NOT EXISTS` — so a database that
-- arrives already carrying an unstemmed `observations_fts`, which is every
-- Leteo store from before the stemmer and every Engram one, keeps the index it
-- came with. Adoption would report success and quietly leave that store
-- searching without a stemmer forever.
--
-- The tokenizer is a property of the table, so the only way to change it is to
-- replace the table. Harmless where the baseline just created it.
--
-- The rebuild also covers every UPDATE above: an external-content FTS5 table
-- does not notice a plain UPDATE, and doing it once at the end costs the same
-- as doing it after each block.
DROP TABLE IF EXISTS observations_fts;
CREATE VIRTUAL TABLE observations_fts USING fts5(
    title, content, tool_name, type, project, topic_key,
    content='observations', content_rowid='id',
    tokenize = 'porter unicode61'
);
INSERT INTO observations_fts(observations_fts) VALUES('rebuild');

DROP TABLE IF EXISTS prompts_fts;
CREATE VIRTUAL TABLE prompts_fts USING fts5(
    content, project, content='prompts', content_rowid='id',
    tokenize = 'porter unicode61'
);
INSERT INTO prompts_fts(prompts_fts) VALUES('rebuild');
