"""Self-checks for session_cost.py: what it counts, how it scores, and that it stays off the stores it is given.

    python3 -m unittest discover -s tools/memory-bench -p 'test_*.py'

No real engine is started. The end-to-end case drives the labels mode with a
stub binary that answers the prompt hook from a fixed rule, which exercises the
copy, the isolated environment, the time cut and the scoring together.
"""
import contextlib
import hashlib
import io
import json
import os
import pathlib
import sqlite3
import stat
import sys
import tempfile
import unittest

import comparison_corpus
import hint_corpus
import session_cost as sc

HERE = pathlib.Path(__file__).resolve().parent


def sha(path):
    return hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def tiny_store(path, rows=()):
    c = sqlite3.connect(path)
    c.executescript("""
        create table observations (id integer primary key, title text, created_at text, deleted_at text);
        create table memory_relations (id integer primary key, created_at text, updated_at text);
    """)
    c.executemany("insert into observations (id, title, created_at, deleted_at) values (?,?,?,?)", rows)
    c.commit()
    c.close()


class Counting(unittest.TestCase):
    def test_a_hook_is_counted_by_its_additional_context_alone(self):
        out = json.dumps({"systemMessage": "for the user", "hookSpecificOutput": {"additionalContext": "for the model"}})
        self.assertEqual(sc.visible_hook_text(out), "for the model")

    def test_plain_text_stdout_is_counted_whole_and_empty_stdout_is_nothing(self):
        self.assertEqual(sc.visible_hook_text("plain text"), "plain text")
        self.assertEqual(sc.visible_hook_text("  \n"), "")
        self.assertEqual(sc.visible_hook_text("{}"), "")

    def test_the_output_schema_is_not_part_of_what_the_model_is_sent(self):
        tool = {"name": "t", "description": "d", "inputSchema": {"type": "object"}, "outputSchema": {"big": "x" * 500}}
        self.assertEqual(sc.model_visible_tool(tool), {"name": "t", "description": "d", "inputSchema": {"type": "object"}})

    def test_the_fallback_unit_says_it_is_not_tokens(self):
        count, unit = sc.make_counter()
        self.assertGreater(count("hello world"), 0)
        self.assertTrue(unit.startswith("tokens") or "bytes/4" in unit)


class Totals(unittest.TestCase):
    def raw(self, nudge=0):
        return {
            "hooks": {"session_start": 100, "prompts": [10, 0, 0, 20], "post_compaction": 50, "nudge": nudge},
            "mcp": {"mem_context": 200, "mem_search": [10, 20, 30], "mem_save": [5, 7, 9], "schema": 1000,
                    "instructions": 40, "tool_count": 3, "output_schema_not_visible": 4000},
        }

    def test_upfront_adds_the_schemas_and_instructions_that_deferred_leaves_out(self):
        r = sc.session_row(self.raw(), engram=False)
        # 100 + 30 + 50 + (200 + 6 * 20 + 3 * 7) = 521
        self.assertEqual(r["total_deferred"], 521)
        self.assertEqual(r["total_upfront"], 521 + 1000 + 40)
        self.assertEqual(r["prompt_hook_injected"], 2)

    def test_engram_pays_one_save_nudge_a_session_and_leteo_none(self):
        self.assertEqual(sc.session_row(self.raw(nudge=34), engram=True)["total_deferred"], 521 + 34)
        self.assertEqual(sc.session_row(self.raw(nudge=34), engram=False)["total_deferred"], 521)


class Labels(unittest.TestCase):
    def write(self, text):
        d = tempfile.mkdtemp()
        p = pathlib.Path(d) / "l.tsv"
        p.write_text(text)
        return p

    HEAD = "id\tcreated_at\tproject\tsource\tprompt\tlabel\tbest_memory_ids\n"

    def test_columns_beyond_the_seventh_are_ignored_and_escapes_are_undone(self):
        p = self.write(self.HEAD + "1\t2026-01-01 00:00:00\tp\treal\tline one\\nline two\trelated\t4,5\textra\tcolumns\n")
        (item,) = sc.read_labels(p)
        self.assertEqual(item["best"], [4, 5])
        self.assertEqual(item["prompt"], "line one\nline two")

    def test_an_unknown_label_is_refused_rather_than_scored_as_neither(self):
        p = self.write(self.HEAD + "1\tt\tp\tsynthetic\thello\tmaybe\t\n")
        with self.assertRaises(SystemExit):
            sc.read_labels(p)

    def test_score_separates_silent_named_and_wrong(self):
        res = [
            dict(label="neutral", best=[], injected=[], tokens=0, bytes=0, rc=0, ms=1),
            dict(label="neutral", best=[], injected=[3], tokens=40, bytes=100, rc=0, ms=2),
            dict(label="related", best=[1], injected=[1, 2], tokens=60, bytes=150, rc=0, ms=3),
            dict(label="related", best=[1], injected=[2], tokens=30, bytes=75, rc=0, ms=4),
            dict(label="related", best=[1], injected=[], tokens=0, bytes=0, rc=1, ms=5),
        ]
        s = sc.score(res)
        self.assertEqual((s["neutral_silent"], s["neutral"]), (1, 2))
        self.assertEqual((s["neutral_spoke"], s["related_named"], s["related"], s["related_spoke_wrong"]), (1, 1, 3, 1))
        self.assertEqual((s["tokens_neutral"], s["tokens_total"], s["hook_failures"]), (40, 130, 1))
        # Four memories named, one of them a labelled one: precision counts every
        # line on every prompt, not only the prompts that had a right memory.
        self.assertEqual((s["named"], s["named_right"], s["precision"]), (4, 1, 0.25))
        self.assertEqual((s["bytes_neutral"], s["bytes_total"]), (100, 325))

    def test_the_example_labels_name_the_memories_the_fixture_builder_writes(self):
        # The fixture is the comparison corpus in write order, so id N is the
        # Nth memory. A corpus edit that moves one breaks the labels silently,
        # and the example would score a hit against the wrong memory.
        titles = {i: m["title"] for i, m in enumerate(comparison_corpus.corpus(), 1)}
        rows = (HERE / "session_cost_data" / "labels.example.tsv").read_text().splitlines()[1:]
        self.assertGreater(len(rows), 20)
        for line in rows:
            c = line.split("\t")
            ids = [int(x) for x in c[6].split(",") if x]
            self.assertEqual(" | ".join(titles[i] for i in ids), c[7], line)
        kinds = {line.split("\t")[3] for line in rows}
        self.assertEqual(kinds, {"synthetic", "real"}, "the example should exercise both replay paths")

    def test_the_hint_labels_name_the_memories_the_hint_fixture_writes(self):
        # The hint fixture is `hint_corpus` in write order, so id N is the Nth
        # memory. The two files are edited apart -- the prompts are prose and the
        # memories are code -- so this is what keeps an insert from renaming a
        # hit to the memory below it.
        titles = {i: m["title"] for i, m in enumerate(hint_corpus.corpus(), 1)}
        rows = (HERE / "session_cost_data" / "labels.hint.tsv").read_text().splitlines()[1:]
        self.assertEqual(len(rows), 40)
        labels = [line.split("\t")[5] for line in rows]
        self.assertEqual((labels.count("related"), labels.count("neutral")), (25, 15))
        for line in rows:
            c = line.split("\t")
            ids = [int(x) for x in c[6].split(",") if x]
            self.assertEqual(" | ".join(titles[i] for i in ids), c[7], line)
            self.assertEqual(c[2], "hint-lab", line)
        self.assertEqual({line.split("\t")[3] for line in rows}, {"synthetic"})

    def test_the_committed_inputs_are_what_the_defaults_read(self):
        data = HERE / "session_cost_data"
        self.assertEqual(len(sc.read_lines(data / "prompts.txt")), 20)
        self.assertGreater(len(sc.read_lines(data / "queries.txt")), 5)
        for s in json.loads((data / "saves.json").read_text()):
            self.assertTrue({"title", "content", "type"} <= set(s))


class Isolation(unittest.TestCase):
    def test_copy_store_leaves_the_original_byte_for_byte(self):
        d = pathlib.Path(tempfile.mkdtemp())
        tiny_store(d / "a.db", [(1, "t", "2026-01-01 00:00:00", None)])
        before = sha(d / "a.db")
        sc.copy_store(d / "a.db", d / "b.db")
        self.assertEqual(sha(d / "a.db"), before)
        self.assertEqual(sqlite3.connect(d / "b.db").execute("select count(*) from observations").fetchone()[0], 1)
        self.assertEqual(sorted(p.name for p in d.iterdir()), ["a.db", "b.db"])

    def test_a_missing_store_is_named_not_created(self):
        d = pathlib.Path(tempfile.mkdtemp())
        with self.assertRaises(SystemExit):
            sc.copy_store(d / "nope.db", d / "b.db")
        self.assertFalse((d / "nope.db").exists())

    def test_the_variables_that_name_a_real_store_do_not_reach_the_engine(self):
        d = pathlib.Path(tempfile.mkdtemp())
        tiny_store(d / "a.db")
        planted = {"LETEO_DATABASE": "/real/leteo.db", "LETEO_DATA_DIR": "/real", "ENGRAM_DATA_DIR": "/real/engram",
                   "ENGRAM_URL": "http://real", "CLAUDE_CONFIG_DIR": "/real/claude"}
        old = {k: os.environ.get(k) for k in planted}
        os.environ.update(planted)
        try:
            for name in ("leteo", "engram"):
                work = d / name
                work.mkdir()
                eng = sc.Engine(name, "bin", d / "a.db", work, "proj")
                for k, v in eng.env.items():
                    self.assertNotIn("/real", v, f"{name}: {k}={v}")
                self.assertEqual(pathlib.Path(eng.env["HOME"]).parent, work)
        finally:
            for k, v in old.items():
                os.environ.pop(k, None)
                if v is not None:
                    os.environ[k] = v


class TimeCut(unittest.TestCase):
    def test_cut_removes_what_did_not_exist_yet_and_revives_what_was_alive(self):
        d = pathlib.Path(tempfile.mkdtemp())
        tiny_store(d / "a.db", [(1, "old", "2026-01-01 00:00:00", None),
                                (2, "new", "2026-03-01 00:00:00", None),
                                (3, "deleted later", "2026-01-02 00:00:00", "2026-03-05 00:00:00"),
                                (4, "deleted before", "2026-01-02 00:00:00", "2026-01-03 00:00:00")])
        sc.cut_store_to(d / "a.db", "2026-02-01 00:00:00")
        rows = sqlite3.connect(d / "a.db").execute("select id, deleted_at is null from observations order by id").fetchall()
        self.assertEqual(rows, [(1, 1), (3, 1), (4, 0)])


STUB = """#!{python}
import json, sys
if sys.argv[1:3] == ["hook", "user-prompt-submit"]:
    p = json.loads(sys.stdin.read())["prompt"]
    ids = [int(w[1:]) for w in p.split() if w.startswith("#")]
    text = "".join(f"- #{{i}} [x] title\\n" for i in ids)
    print(json.dumps({{"hookSpecificOutput": {{"additionalContext": text}}}}) if ids else "")
"""


class EndToEnd(unittest.TestCase):
    def test_labels_mode_replays_a_stub_hook_over_a_copy_and_scores_it(self):
        d = pathlib.Path(tempfile.mkdtemp())
        stub = d / "leteo"
        stub.write_text(STUB.format(python=sys.executable))
        stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
        tiny_store(d / "store.db", [(1, "m", "2026-01-01 00:00:00", None)])
        before = sha(d / "store.db")
        (d / "l.tsv").write_text(
            "id\tcreated_at\tproject\tsource\tprompt\tlabel\tbest_memory_ids\n"
            "s1\tx\tp\tsynthetic\tthe answer is #1\trelated\t1\n"
            "s2\tx\tp\tsynthetic\tok thanks\tneutral\t\n"
            "s3\tx\tp\tsynthetic\tnoise #9\tneutral\t\n"
            "1\t2026-02-01 00:00:00\tp\treal\tcut then ask #1\trelated\t1\n")
        out = d / "out.json"
        sc.main(["--leteo-bin", str(stub), "--leteo-store", str(d / "store.db"),
                 "--labels", str(d / "l.tsv"), "--json", str(out)])
        s = json.loads(out.read_text())["score"]
        self.assertEqual((s["neutral_silent"], s["neutral"]), (1, 2))
        self.assertEqual((s["related_named"], s["related"]), (2, 2))
        self.assertEqual(s["hook_failures"], 0)
        self.assertEqual(sha(d / "store.db"), before, "the store that was given must not change")


FAILING_STUB = """#!{python}
import sys
sys.stderr.write("boom: {why}\\nsecond line\\n")
sys.exit(3)
"""


class HookFailure(unittest.TestCase):
    """A hook that exits non-zero has produced no measurement, not a quiet one."""

    def setup(self, why):
        d = pathlib.Path(tempfile.mkdtemp())
        stub = d / "leteo"
        stub.write_text(FAILING_STUB.format(python=sys.executable, why=why))
        stub.chmod(stub.stat().st_mode | stat.S_IEXEC)
        tiny_store(d / "store.db")
        return d, stub

    def run_failing(self, d, stub, extra):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            with self.assertRaises(SystemExit) as ctx:
                sc.main(["--leteo-bin", str(stub), "--leteo-store", str(d / "store.db"), *extra])
        return str(ctx.exception.code), buf.getvalue()

    def test_session_mode_names_the_failed_hook_and_prints_no_table(self):
        d, stub = self.setup("session")
        prompts = d / "p.txt"
        prompts.write_text("one prompt\n")
        msg, out = self.run_failing(d, stub, ["--prompts", str(prompts)])
        self.assertIn("leteo: start hook exited 3: boom: session", msg)
        self.assertIn("one prompt", msg)
        self.assertNotIn("second line", msg)
        self.assertEqual(out, "", "no table may be printed for a measurement with a failed hook")

    def test_labels_mode_names_the_failed_prompt_and_prints_no_score(self):
        d, stub = self.setup("labels")
        (d / "l.tsv").write_text(
            "id\tcreated_at\tproject\tsource\tprompt\tlabel\tbest_memory_ids\n"
            "s2\tx\tp\tsynthetic\tok thanks\tneutral\t\n")
        msg, out = self.run_failing(d, stub, ["--labels", str(d / "l.tsv")])
        self.assertIn("prompt 'ok thanks' exited 3: boom: labels", msg)
        self.assertEqual(out, "")


if __name__ == "__main__":
    unittest.main()
