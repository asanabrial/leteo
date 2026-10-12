"""Self-checks for compare.py's tables, its not-measured path and its pass handling.

    python3 -m unittest discover -s tools/memory-bench -p 'test_*.py'

No engine is started here: every table is built from a fixed list of rows, and
`run_repeated` is driven by an adapter that only loads.
"""
import os
import shutil
import tempfile
import unittest

os.environ.setdefault("BENCH_STATE", os.path.join(tempfile.gettempdir(), "leteo-adapters-selfcheck"))

import bench_engines  # noqa: E402
import compare  # noqa: E402
from adapters import Adapter, Skipped  # noqa: E402
from comparison_corpus import corpus  # noqa: E402


def row(kind="k", rank=None, n=20, unmappable=0):
    return dict(kind=kind, rank=rank, n=n, unmappable=unmappable)


def engine(label, rows, no_answer=(), status="measured", version="1.0", date="2026-01-01", reason=""):
    return dict(name=label.lower(), label=label, version=version, date=date, status=status,
                reason=reason, rows=list(rows), no_answer=list(no_answer))


def measured(label, passes):
    """An engine measured over several loads: the first pass's rows, and all of them."""
    return dict(engine(label, passes[0]["rows"], passes[0]["no_answer"]), passes=passes)


def one(ranks):
    return dict(rows=[row(rank=r) for r in ranks], no_answer=[])


class Figure(unittest.TestCase):
    def test_one_pass_is_its_own_number(self):
        self.assertEqual(compare.figure([0.750], 3), "0.750")

    def test_several_passes_are_a_mean_with_the_range(self):
        self.assertEqual(compare.figure([0.750, 0.500], 3), "0.625 (0.500–0.750)")

    def test_passes_that_agree_say_every_pass(self):
        self.assertEqual(compare.figure([0.750, 0.750], 3), "0.750 (every pass)")

    def test_a_spread_finer_than_the_printed_digits_widens_instead(self):
        self.assertEqual(compare.figure([0.4996, 0.5004], 3), "0.5000 (0.4996–0.5004)")


class EnginesTable(unittest.TestCase):
    def test_a_measured_engine_is_a_row_with_its_version_and_date(self):
        text = compare.engines_table([engine("Leteo", [row(rank=1)])])
        self.assertIn("| Leteo | 1.0 | 2026-01-01 | measured |", text)

    def test_a_skipped_engine_names_the_prerequisite_that_was_absent(self):
        e = engine("basic-memory", [], status="skipped", version="", reason="`uvx` was not found on PATH")
        text = compare.engines_table([e])
        self.assertIn("| basic-memory | — | 2026-01-01 | **not measured:** `uvx` was not found on PATH |", text)

    def test_a_crashed_engine_is_failed_and_not_read_as_a_missing_prerequisite(self):
        e = engine("basic-memory", [], status="failed", version="",
                   reason="could not run: RuntimeError: store unreadable")
        text = compare.engines_table([e])
        self.assertIn("| basic-memory | — | 2026-01-01 | **failed:** could not run: RuntimeError: store unreadable |", text)

    def test_a_row_measured_over_passes_names_the_count(self):
        e = measured("basic-memory", [one([1]) for _ in range(5)])
        self.assertIn("| basic-memory | 1.0 | 2026-01-01 | measured, mean of 5 passes |", compare.engines_table([e]))


class OverallTable(unittest.TestCase):
    def test_every_engine_gets_a_row_of_its_own_numbers(self):
        a = engine("A", [row(rank=1), row(rank=2, n=5, unmappable=1)], no_answer=[dict(n=0), dict(n=3)])
        b = engine("B", [row(rank=None, n=0)])
        text = compare.overall_table([a, b])
        self.assertIn("| A | 1.0 | 2026-01-01 | 0.50 | 1.00 | 0.750 | 0 of 2 | 25 | 1 | 1 of 2 |", text)
        self.assertIn("| B | 1.0 | 2026-01-01 | 0.00 | 0.00 | 0.000 | 1 of 1 | 0 | 0 | 0 of 0 |", text)

    def test_a_row_measured_over_passes_reports_the_spread(self):
        text = compare.overall_table([measured("basic-memory", [one([1, 2]), one([1, None])])])
        self.assertIn("| 0.625 (0.500–0.750) |", text)


class KindTable(unittest.TestCase):
    def test_each_engine_is_a_column_and_the_count_comes_from_the_first(self):
        a = engine("A", [row("k1", 1), row("k2", 2)])
        b = engine("B", [row("k1", 1), row("k2", 1)])
        text = compare.kind_table([a, b])
        self.assertIn("| query kind | n | A h@1 / h@5 / MRR | B h@1 / h@5 / MRR |", text)
        self.assertIn("| k1 | 1 | 1.00 / 1.00 / 1.000 | 1.00 / 1.00 / 1.000 |", text)
        self.assertIn("| **overall** | 2 | 0.50 / 1.00 / 0.750 | 1.00 / 1.00 / 1.000 |", text)

    def test_a_multi_pass_engine_shows_the_mean_of_its_passes(self):
        a = measured("A", [dict(rows=[row("k2", 2)], no_answer=[]),
                           dict(rows=[row("k2", 1)], no_answer=[])])
        self.assertIn("| k2 | 1 | 0.50 / 1.00 / 0.750 |", compare.kind_table([a]))


class LeadsTable(unittest.TestCase):
    def test_a_tie_names_every_engine_that_holds_the_top(self):
        text = compare.leads_table([engine("A", [row(rank=1)]), engine("B", [row(rank=1)])], {})
        self.assertIn("tie A, B 1.000", text)

    def test_a_lone_leader_is_named_with_its_number(self):
        text = compare.leads_table([engine("A", [row(rank=1)]), engine("B", [row(rank=2)])], {})
        self.assertIn("| A 1.000 |", text)
        self.assertNotIn("tie", text)

    def test_the_leader_uses_the_mean_of_the_passes(self):
        leteo = engine("leteo", [row(rank=1), row(rank=1)])
        basic = measured("basic-memory", [one([1, 1]), one([1, None])])
        self.assertIn("leteo 1.000", compare.leads_table([leteo, basic], {}))


class NothingMeasured(unittest.TestCase):
    def test_the_kind_table_does_not_read_an_engine_that_is_not_there(self):
        self.assertIn("query kind", compare.kind_table([]))

    def test_the_leads_table_does_not_rank_an_empty_field(self):
        self.assertIn("query kind", compare.leads_table([], {}))


class FakeAdapter(Adapter):
    """The smallest engine `run_repeated` can drive: it loads, answers nothing."""

    name, label = "fake", "fake"
    calls = 0
    fail_on = 0
    crash_on = 0
    checks = 0
    check_fail_on = 0
    closes = 0
    close_fail_on = 0
    states = []

    def check(self):
        FakeAdapter.checks += 1
        if FakeAdapter.checks == FakeAdapter.check_fail_on:
            raise Skipped("`uvx` was not found on PATH")

    def version(self):
        return "0"

    def load(self, members):
        FakeAdapter.calls += 1
        FakeAdapter.states.append(self.state)
        if FakeAdapter.calls == FakeAdapter.fail_on:
            raise Skipped("no engine")
        if FakeAdapter.calls == FakeAdapter.crash_on:
            raise RuntimeError("store unreadable")

    def search(self, project, query, limit):
        return []

    def close(self):
        FakeAdapter.closes += 1
        if FakeAdapter.closes == FakeAdapter.close_fail_on:
            raise RuntimeError("store left open")


class RunRepeated(unittest.TestCase):
    def setUp(self):
        FakeAdapter.calls = 0
        FakeAdapter.fail_on = 0
        FakeAdapter.crash_on = 0
        FakeAdapter.checks = 0
        FakeAdapter.check_fail_on = 0
        FakeAdapter.closes = 0
        FakeAdapter.close_fail_on = 0
        FakeAdapter.states = []
        self.state = tempfile.mkdtemp(prefix="leteo-run-repeated-")
        self.addCleanup(shutil.rmtree, self.state, ignore_errors=True)

    def test_a_complete_run_keeps_every_pass_and_the_first_passs_rows(self):
        entry = bench_engines.run_repeated(FakeAdapter, self.state, corpus(), "2026-01-01", 3)
        self.assertEqual(len(entry["passes"]), 3)
        self.assertEqual(entry["status"], "measured")
        self.assertTrue(entry["rows"])
        self.assertEqual(entry["rows"], entry["passes"][0]["rows"])
        self.assertIn("mean of 3 passes", compare.engines_table([entry]))

    def test_every_pass_is_given_a_state_of_its_own(self):
        bench_engines.run_repeated(FakeAdapter, self.state, corpus(), "2026-01-01", 3)
        self.assertEqual(len(set(FakeAdapter.states)), 3)

    def test_a_prerequisite_that_vanishes_after_a_pass_is_a_fault_not_a_skip(self):
        FakeAdapter.fail_on = 2
        entry = bench_engines.run_repeated(FakeAdapter, self.state, corpus(), "2026-01-01", 3)
        self.assertEqual(entry["status"], "failed")
        self.assertEqual(entry["passes"], [])
        self.assertEqual(entry["rows"], [])
        self.assertIn("stopped after 1 of 3 passes", entry["reason"])
        self.assertIn("**failed:**", compare.engines_table([entry]))

    def test_a_close_that_raises_ends_the_run_and_names_the_fault(self):
        FakeAdapter.close_fail_on = 2
        entry = bench_engines.run_repeated(FakeAdapter, self.state, corpus(), "2026-01-01", 3)
        self.assertEqual(entry["status"], "failed")
        self.assertEqual(entry["rows"], [])
        self.assertIn("store left open", entry["reason"])
        self.assertIn("**failed:**", compare.engines_table([entry]))

    def test_a_prerequisite_check_reports_is_skipped(self):
        FakeAdapter.check_fail_on = 1
        entry = bench_engines.run_repeated(FakeAdapter, self.state, corpus(), "2026-01-01", 3)
        self.assertEqual(entry["passes"], [])
        self.assertEqual(entry["status"], "skipped")
        self.assertIn("`uvx` was not found on PATH", entry["reason"])
        self.assertIn("**not measured:**", compare.engines_table([entry]))

    def test_a_crash_on_the_first_pass_is_failed_not_skipped(self):
        FakeAdapter.crash_on = 1
        entry = bench_engines.run_repeated(FakeAdapter, self.state, corpus(), "2026-01-01", 3)
        self.assertEqual(entry["passes"], [])
        self.assertEqual(entry["status"], "failed")
        self.assertIn("could not run:", entry["reason"])
        self.assertIn("**failed:**", compare.engines_table([entry]))


if __name__ == "__main__":
    unittest.main()
