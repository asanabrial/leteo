# Schema migrations

The local SQLite schema is versioned with `PRAGMA user_version`, a integer that
lives in the database header and costs nothing to read.

## Adding a change

1. Write a new file, numbered above the last one:
   `migrations/0002_add_something.sql`.
2. Register it in `MIGRATIONS` in `src/store.rs`.
3. Bump `SCHEMA_VERSION` to the same number.

That is the whole procedure. A test refuses to build a release where the
numbers do not ascend or where `SCHEMA_VERSION` disagrees with the last file.

## The one rule

**A released migration is never edited.** Databases that already ran it will
not run it again, so changing the file only changes what *new* databases get,
and quietly splits the two populations apart. Fix a mistake by adding another
migration.

The word doing the work is **released**. Before the first release the ten
migrations that had accumulated — 7 through 16, each written the day something
was found — were collapsed into one file, because no database outside this
repository had run any of them and there were no two populations to split. It
kept the highest number rather than the lowest: a store stamped anywhere below
16 runs the whole file and arrives at the same shape, while renumbering down to
7 would have left every store between 8 and 15 behind without a word. What made
it safe is that every statement in it is idempotent — the indexes and the
virtual table are `IF NOT EXISTS`, and the four that rewrite data each fold a
value onto a canonical one.

That was a one-off. From here the rule above applies as written.

## What happens on open

| Database is | What happens |
| --- | --- |
| Stamped above `SCHEMA_VERSION` | Refused, saying which version it found |
| Unstamped (`user_version = 0`) | Adopted, then stamped 1 |
| Stamped at `k` | Every migration above `k` applied in order |

Refusing a database from the future is the point of the whole scheme: an older
binary cannot know what a newer one changed, and would write the shape it
remembers over the top of it.

## Leteo's names are Leteo's

The schema follows Leteo's needs. It is not held to the names of the project it
was reimplemented from, and the two are expected to drift apart.

Compatibility lives in the adapter instead: `src/engram.rs` holds the only
mapping between the two vocabularies, and adoption translates rather than
copies. That is the right place for it — a storage layer frozen to somebody
else's naming would pay for that compatibility forever, in every query.

## Why adoption exists

Leteo opens databases it did not create — an Engram store, an early Engram
schema, or a Leteo database written before any of this existed. None of them
record a version, so there is no step to resume from. `adopt_to_baseline` in `src/store.rs` inspects what is actually present and
converges it on version 1.

**That function is frozen.** It is the only part of migration that grows by
inspection rather than by number, and it stays fixed at the shape of version 1
so it never becomes the sprawling thing this layout exists to avoid.

## Version 1 is two files

`0001_baseline_tables.sql` creates the tables; `0001_baseline_finalize.sql`
adds the backfills, indexes, and full-text triggers. They are separate because
the triggers reference columns that a legacy database only gains during
adoption, so they cannot run until that has happened.

## What catches a mistake, given there is no ORM

SQL is strings, so nothing checks a query against the schema at build time. A
column that does not exist fails when the query runs. Three tests stand in for
what an ORM would give:

- `the_migrated_schema_is_exactly_what_the_queries_expect` asserts the full
  column set of each table — not just that what we read is present, but that
  nothing else is.
- `every_shared_column_list_is_valid_sql_against_the_real_schema` prepares each
  shared column list against a migrated database, so a typo fails a test rather
  than a user's search.
- `the_joined_observation_columns_expose_the_same_names` keeps the plain and
  join-qualified lists in step.

The first one matters more than it looks. Adoption is deliberately forgiving:
rename a column in a migration and `add_column_if_missing` quietly adds the old
name back, leaving **both**. Everything compiles, every other test passes, and
the database carries a dead column. Only asserting the exact set catches it.

## Not the same as the export format

`EXPORT_FORMAT_VERSION` versions the JSON that `leteo export` writes. The two
move independently: the storage layout can change without the interchange
format changing, and the reverse.
