"""Self-checks for the parts of the adapters that decide what a hit is.

    python3 -m unittest discover -s tools/memory-bench -p 'test_adapters.py'
"""
import os
import tempfile
import unittest

os.environ.setdefault("BENCH_STATE", os.path.join(tempfile.gettempdir(), "leteo-adapters-selfcheck"))

import adapters  # noqa: E402
import bench_engines  # noqa: E402
from adapters import Adapter, Hit, Skipped, content_key, isolated_env, normalise, stored_titles  # noqa: E402

LONG = "Offset pagination re-read the whole table for deep pages. Orders now page by a keyset cursor."
MEMBERS = [
    dict(key="keyset", content=LONG),
    dict(key="retry", content="Clients retry a failed call with exponential backoff, a 200 ms base and a 30 s cap."),
]


class ContentRule(unittest.TestCase):
    def test_the_saved_text_is_the_memory(self):
        self.assertEqual(content_key(LONG, MEMBERS), "keyset")

    def test_a_rewrapped_recased_snippet_is_still_the_memory(self):
        self.assertEqual(content_key("offset PAGINATION re-read the whole table\nfor deep pages", MEMBERS), "keyset")

    def test_a_short_snippet_is_nothing(self):
        self.assertIsNone(content_key("keyset cursor", MEMBERS))

    def test_text_that_is_no_memory_is_nothing(self):
        self.assertIsNone(content_key("A paragraph about parental leave that no memory holds at all.", MEMBERS))

    def test_a_snippet_two_memories_share_is_not_guessed(self):
        twins = MEMBERS + [dict(key="twin", content=LONG + " Same text, another memory.")]
        self.assertIsNone(content_key("offset pagination re-read the whole table for deep pages", twins))

    def test_normalise_folds_accents_and_punctuation(self):
        self.assertEqual(normalise("Cuál  es-la política"), "cual es la politica")


class Mapping(unittest.TestCase):
    def adapter(self):
        a = Adapter(tempfile.gettempdir())
        a.keymap = {"7": "keyset"}
        return a

    def test_provenance_wins_over_content(self):
        self.assertEqual(self.adapter().map_hit(Hit(7, "unrelated"), MEMBERS), "keyset")

    def test_a_reference_we_never_wrote_is_unmappable_even_when_the_text_fits(self):
        self.assertIsNone(self.adapter().map_hit(Hit(99, LONG), MEMBERS))

    def test_a_hit_without_a_reference_falls_back_to_content(self):
        self.assertEqual(self.adapter().map_hit(Hit(None, LONG), MEMBERS), "keyset")


class Isolation(unittest.TestCase):
    def test_every_store_variable_is_replaced_or_removed_and_home_is_under_state(self):
        for var in adapters.STORE_VARIABLES:
            os.environ[var] = "/home/real/store"
        try:
            with tempfile.TemporaryDirectory() as state:
                env = isolated_env(state, "x")
                for var in adapters.STORE_VARIABLES:
                    self.assertNotEqual(env.get(var), "/home/real/store", var)
                for var in ("HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
                    self.assertTrue(env[var].startswith(state), var)
        finally:
            for var in adapters.STORE_VARIABLES:
                os.environ.pop(var, None)


class Naming(unittest.TestCase):
    def test_a_repeated_title_is_made_unique_within_its_project_only(self):
        ms = [dict(key="a", project="p", title="T"), dict(key="b", project="p", title="T"),
              dict(key="c", project="q", title="T")]
        self.assertEqual(stored_titles(ms), {"a": "T", "b": "T (2)", "c": "T"})


class Skipping(unittest.TestCase):
    def test_a_missing_runner_skips_the_engine_with_its_name_and_no_rows(self):
        old = os.environ["PATH"]
        os.environ["PATH"] = ""
        try:
            entry = bench_engines.run_one(adapters.BasicMemoryAdapter, tempfile.gettempdir(), [], "2026-01-01")
        finally:
            os.environ["PATH"] = old
        self.assertEqual(entry["status"], "skipped")
        self.assertIn("`uvx` was not found on PATH", entry["reason"])
        self.assertEqual(entry["rows"], [])

    def test_an_engine_that_starts_and_then_faults_is_failed_and_never_scored(self):
        class Broken(adapters.ServerMemoryAdapter):
            def check(self):
                pass

            def version(self):
                return "0"

            def load(self, members):
                raise RuntimeError("store unreadable")

        entry = bench_engines.run_one(Broken, tempfile.gettempdir(), [], "2026-01-01")
        self.assertEqual(entry["status"], "failed")
        self.assertIn("store unreadable", entry["reason"])
        self.assertEqual(entry["rows"], [])



class Teardown(unittest.TestCase):
    """A store that cannot be closed is a fault, not a run that vanishes."""

    class Broken(adapters.ServerMemoryAdapter):
        def check(self):
            pass

        def version(self):
            return "0"

        def load(self, members):
            pass

        def close(self):
            raise RuntimeError("store left open")

    def run_one(self, cls):
        saved = (bench_engines.queries, bench_engines.no_answer)
        bench_engines.queries, bench_engines.no_answer = lambda: [], lambda: []
        try:
            return bench_engines.run_one(cls, tempfile.gettempdir(), [], "2026-01-01")
        finally:
            bench_engines.queries, bench_engines.no_answer = saved

    def test_a_close_that_raises_is_named_and_the_entry_survives(self):
        entry = self.run_one(self.Broken)
        self.assertEqual(entry["status"], "failed")
        self.assertIn("store left open", entry["reason"])
        self.assertEqual(entry["reason"].count("store left open"), 1)
        self.assertEqual(entry["rows"], [])

    def test_a_close_fault_is_named_beside_the_fault_the_run_had(self):
        class LoadAndClose(self.Broken):
            def load(self, members):
                raise RuntimeError("store unreadable")

        entry = self.run_one(LoadAndClose)
        self.assertEqual(entry["status"], "failed")
        self.assertIn("store unreadable", entry["reason"])
        self.assertIn("store left open", entry["reason"])


class LeteoNeedsThisRunsEngram(unittest.TestCase):
    def test_a_keymap_left_by_an_earlier_run_is_not_an_engram_load(self):
        with tempfile.TemporaryDirectory() as bench:
            open(os.path.join(bench, "keymap_engram.json"), "w").write("{}")
            saved, adapters.BENCH = adapters.BENCH, bench
            adapters.LOADED_THIS_RUN.clear()
            try:
                with self.assertRaises(Skipped) as caught:
                    adapters.LeteoAdapter(bench).load(MEMBERS)
            finally:
                adapters.BENCH = saved
        self.assertIn("not loaded in this run", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
