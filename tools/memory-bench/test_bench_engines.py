"""Self-checks for bench_engines: the metric, the crash status and the refusals.

    python3 -m unittest discover -s tools/memory-bench -p 'test_*.py'

No engine is started and no store is read: `measure` runs over a fake adapter and
`main` over a fake tier.
"""
import contextlib
import io
import json
import os
import tempfile
import unittest

os.environ.setdefault("BENCH_STATE", os.path.join(tempfile.gettempdir(), "leteo-adapters-selfcheck"))

import bench_engines  # noqa: E402
import llm_tier  # noqa: E402


class Measure(unittest.TestCase):
    MEMBERS = [dict(key="t", project="p"), dict(key="u", project="p")]

    def measure(self, answers, no_answers=None):
        """Run `bench_engines.measure` over a fake engine whose hits are keys.

        A hit is whatever the fake `map_hit` returns: `"t"` maps to the target,
        `None` is unmappable. The search answers are keyed by the query text.
        """
        qs = [dict(kind="k", q=key, target="t", project="p") for key in answers]
        nas = [dict(kind="no_answer", q=key, project="p") for key in (no_answers or {})]
        replies = dict(answers, **(no_answers or {}))
        limits = []

        class Fake:
            def search(self, project, query, limit):
                limits.append(limit)
                return list(replies[query])

            def map_hit(self, hit, members):
                return hit

        saved = (bench_engines.queries, bench_engines.no_answer)
        bench_engines.queries = lambda: qs
        bench_engines.no_answer = lambda: nas
        try:
            rows, na = bench_engines.measure(Fake(), self.MEMBERS)
        finally:
            bench_engines.queries, bench_engines.no_answer = saved
        return rows, na, limits

    def test_an_unmappable_hit_still_holds_the_place_it_took(self):
        rows, _, _ = self.measure({"q": [None, "t", None]})
        self.assertEqual((rows[0]["rank"], rows[0]["n"], rows[0]["unmappable"]), (2, 3, 2))

    def test_hits_past_the_limit_are_cut_and_cannot_rank(self):
        many = [None] * bench_engines.LIMIT + ["t"]
        rows, _, limits = self.measure({"q": many})
        self.assertEqual(rows[0]["n"], bench_engines.LIMIT)
        self.assertIsNone(rows[0]["rank"])
        self.assertEqual(limits, [bench_engines.LIMIT])

    def test_an_empty_reply_is_a_query_that_returned_nothing(self):
        rows, _, _ = self.measure({"q": []})
        self.assertEqual((rows[0]["n"], rows[0]["rank"], rows[0]["unmappable"]), (0, None, 0))

    def test_an_unanswerable_query_records_how_many_hits_came_back(self):
        _, na, _ = self.measure({"q": []}, no_answers={"y": [None, "t"]})
        self.assertEqual(na[0]["n"], 2)


class FakeMeasured:
    name, label, required = "fake", "Fake", False

    def __init__(self, state):
        pass


class FakeRequired:
    name, label, required = "req", "Required", True

    def __init__(self, state):
        pass


MEASURED = dict(name="fake", label="Fake", date="2026-01-01", required=False, status="measured",
                reason="", version="", rows=[], no_answer=[])
SKIPPED_REQUIRED = dict(name="req", label="Required", date="2026-01-01", required=True, status="skipped",
                        reason="`uvx` was not found on PATH", version="", rows=[], no_answer=[])
SKIPPED_OPTIONAL = dict(name="fake", label="Fake", date="2026-01-01", required=False, status="skipped",
                        reason="Ollama is not reachable at http://localhost:11434", version="", rows=[], no_answer=[])


class Refusals(unittest.TestCase):
    def run_main(self, argv, adapters, entries):
        """`bench_engines.main` over a fake tier, each engine answering its own card."""
        saved = (bench_engines.DETERMINISTIC, bench_engines.run_one, bench_engines.run_llm,
                 bench_engines.corpus, bench_engines.BENCH, llm_tier.LLM)

        def card(cls):
            return dict(entries[cls.name])

        with tempfile.TemporaryDirectory() as state:
            bench_engines.DETERMINISTIC = tuple(adapters)
            bench_engines.run_one = lambda cls, s, m, d: card(cls)
            bench_engines.run_llm = lambda cls, s, m, d, p: card(cls)
            bench_engines.corpus = lambda: []
            bench_engines.BENCH = state
            llm_tier.LLM = tuple(adapters)
            try:
                with contextlib.redirect_stderr(io.StringIO()):
                    return bench_engines.main(argv)
            finally:
                (bench_engines.DETERMINISTIC, bench_engines.run_one, bench_engines.run_llm,
                 bench_engines.corpus, bench_engines.BENCH, llm_tier.LLM) = saved

    def test_a_required_engine_that_was_not_measured_refuses_the_run(self):
        # A second engine measures, so this refusal can only come from `missing`.
        entries = {"fake": MEASURED, "req": SKIPPED_REQUIRED}
        self.assertEqual(self.run_main(["--only", "fake,req"], [FakeMeasured, FakeRequired], entries), 2)

    def test_a_run_that_measured_something_is_not_refused(self):
        self.assertEqual(self.run_main(["--only", "fake"], [FakeMeasured], {"fake": MEASURED}), 0)

    def test_a_tier_that_produced_no_engine_refuses_the_run(self):
        self.assertEqual(self.run_main([], [], {}), 2)

    def test_the_llm_tier_that_measured_no_engine_refuses_the_run(self):
        # Every LLM-tier engine is non-required, so only the empty-run guard refuses this.
        self.assertEqual(self.run_main(["--tier", "llm"], [FakeMeasured], {"fake": SKIPPED_OPTIONAL}), 2)


class FakeLLM:
    """The smallest LLM-tier engine `run_llm` can drive: it loads, answers nothing."""

    name, label = "fake-llm", "Fake LLM"
    loads = 0
    fail_on = 0
    closes = 0
    close_fail_on = 0

    def __init__(self, state):
        self.notes = []

    def check(self):
        pass

    def version(self):
        return "0"

    def models(self):
        return {}

    @property
    def ollama(self):
        return {"version": "0"}

    def load(self, members):
        FakeLLM.loads += 1
        if FakeLLM.loads == FakeLLM.fail_on:
            raise RuntimeError("model did not load")

    def search(self, project, query, limit):
        return []

    def close(self):
        FakeLLM.closes += 1
        if FakeLLM.closes == FakeLLM.close_fail_on:
            raise RuntimeError("store left open")


class LLMTeardown(unittest.TestCase):
    def run_llm(self, passes=2, fail_on=0, close_fail_on=0):
        FakeLLM.loads = FakeLLM.closes = 0
        FakeLLM.fail_on, FakeLLM.close_fail_on = fail_on, close_fail_on
        saved = (bench_engines.queries, bench_engines.no_answer)
        bench_engines.queries, bench_engines.no_answer = lambda: [], lambda: []
        with tempfile.TemporaryDirectory() as state:
            try:
                return bench_engines.run_llm(FakeLLM, state, [], "2026-01-01", passes)
            finally:
                bench_engines.queries, bench_engines.no_answer = saved

    def test_a_close_that_raises_is_failed_even_though_the_passes_ran(self):
        entry = self.run_llm(passes=3, close_fail_on=1)
        self.assertEqual(entry["status"], "failed")
        self.assertEqual(entry["passes"], [])
        self.assertIn("store left open", entry["reason"])

    def test_a_close_fault_beside_a_pass_fault_still_discards_the_partial_run(self):
        entry = self.run_llm(passes=3, fail_on=2, close_fail_on=2)
        self.assertEqual(entry["passes"], [])
        self.assertEqual(entry["status"], "failed")
        self.assertIn("model did not load", entry["failed"])
        self.assertIn("store left open", entry["failed"])


class MultiEngineTeardown(unittest.TestCase):
    """A close fault in one engine does not take the other engines' rows with it."""

    class Healthy(FakeMeasured):
        name, label = "healthy", "Healthy"

        def check(self):
            pass

        def version(self):
            return "0"

        def load(self, members):
            pass

        def search(self, project, query, limit):
            return []

        def close(self):
            pass

    class Torn(Healthy):
        name, label = "torn", "Torn"

        def close(self):
            raise RuntimeError("store left open")

    def test_the_other_engines_keep_their_rows_and_the_page_is_written(self):
        saved = (bench_engines.DETERMINISTIC, bench_engines.corpus, bench_engines.BENCH,
                 bench_engines.queries, bench_engines.no_answer)
        with tempfile.TemporaryDirectory() as state:
            bench_engines.DETERMINISTIC = (self.Healthy, self.Torn)
            bench_engines.corpus = lambda: []
            bench_engines.BENCH = state
            bench_engines.queries, bench_engines.no_answer = lambda: [], lambda: []
            try:
                with contextlib.redirect_stderr(io.StringIO()):
                    code = bench_engines.main([])
            finally:
                (bench_engines.DETERMINISTIC, bench_engines.corpus, bench_engines.BENCH,
                 bench_engines.queries, bench_engines.no_answer) = saved
            written = json.load(open(os.path.join(state, "results_engines.json")))
        self.assertEqual(code, 0)
        self.assertEqual([e["status"] for e in written["engines"]], ["measured", "failed"])
        self.assertIn("store left open", written["engines"][1]["reason"])


if __name__ == "__main__":
    unittest.main()
