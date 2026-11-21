-- A memory is stemmed in the language it is written in.
--
-- Migrations 21 and 22 recorded a row under `leteo_stem_language()`, the
-- connection's language, and the connection's language is English whenever the
-- `language` setting names none, which is the default. On one real store every
-- one of 7,274 rows was recorded `en` while about 1,500 of the memories were
-- written in Spanish, so the Snowball index was inert: it stems English as
-- nothing, and a question in Spanish with another ending than the memory's
-- found nothing.
--
-- `leteo_memory_language(title, content)` is the row's language. It answers the
-- setting's language when the setting names one, and otherwise the language the
-- words of the memory are in (`stemming::detect`). The two triggers that write
-- stems call it once and hand the answer to `leteo_stem` for every column, so a
-- row is never recorded under one language and stemmed in another and the text
-- is read once.
--
-- The `WHERE true` is what lets an `INSERT ... SELECT` carry an upsert: without
-- it SQLite reads `ON CONFLICT` as part of the join.
--
-- `obs_stems_identifiers` is not touched. It refreshes copies of identifiers and
-- keeps the language and the stems, which is what migration 22 made it for.
--
-- No row is rewritten here. A migration cannot say what an existing row's
-- language was meant to be, and the stems of every row are a function of the
-- row: `leteo doctor --repair` re-stems the ones whose recorded language is not
-- the one their words are in. No column a full-text index carries changes, so no
-- index is rebuilt.

DROP TRIGGER IF EXISTS obs_stems_insert;
DROP TRIGGER IF EXISTS obs_stems_update;

CREATE TRIGGER obs_stems_insert AFTER INSERT ON observations BEGIN
    INSERT INTO observation_stems(observation_id, language, title, content, tool_name, type, project, topic_key)
    SELECT new.id, chosen.code, leteo_stem(new.title, chosen.code), leteo_stem(new.content, chosen.code), new.tool_name, new.type, new.project, leteo_stem(new.topic_key, chosen.code)
    FROM (SELECT leteo_memory_language(new.title, new.content) AS code) AS chosen WHERE true
    ON CONFLICT(observation_id) DO UPDATE SET language = excluded.language, title = excluded.title, content = excluded.content, tool_name = excluded.tool_name, type = excluded.type, project = excluded.project, topic_key = excluded.topic_key;
END;
CREATE TRIGGER obs_stems_update AFTER UPDATE OF title, content, topic_key ON observations
WHEN new.title IS NOT old.title OR new.content IS NOT old.content OR new.topic_key IS NOT old.topic_key BEGIN
    INSERT INTO observation_stems(observation_id, language, title, content, tool_name, type, project, topic_key)
    SELECT new.id, chosen.code, leteo_stem(new.title, chosen.code), leteo_stem(new.content, chosen.code), new.tool_name, new.type, new.project, leteo_stem(new.topic_key, chosen.code)
    FROM (SELECT leteo_memory_language(new.title, new.content) AS code) AS chosen WHERE true
    ON CONFLICT(observation_id) DO UPDATE SET language = excluded.language, title = excluded.title, content = excluded.content, tool_name = excluded.tool_name, type = excluded.type, project = excluded.project, topic_key = excluded.topic_key;
END;
