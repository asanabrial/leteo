"""Self-checks for the paired comparison: the pairing, the two drops, the resolution.

    python3 -m unittest discover -s tools/memory-bench -p 'test_*.py'

No binary is needed for the comparison itself: every case builds the two runs'
rows by hand. The one test that starts a binary — the product's default row
count against what a search with no `limit` returns — skips when `LETEO_BIN`
is unset, because the fallback is whatever `leteo` is on PATH and a check
against an installed release is not a check of this build.
"""
import argparse
import io
import json
import os
import subprocess
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest import mock

os.environ.setdefault("BENCH_STATE", os.path.join(tempfile.gettempdir(), "leteo-precision-selfcheck"))

import precision  # noqa: E402
import quality  # noqa: E402
from precision import (  # noqa: E402
    CannotRun,
    compare,
    corpora_from,
    limits_from,
    rank_worse,
    resolution,
    search_arguments,
)


def row(limit=10, kind="short", q="a question", project="alpha-api", rank=1, size=100, mode=None):
    return dict(
        kind=kind, q=q, project=project, limit=limit, mode=mode, rank=rank, results=20, bytes=size
    )


def questions(count=4, limit=10, rank=None):
    """`count` questions, all at `rank` unless `rank` is a list."""
    ranks = rank if isinstance(rank, list) else [rank] * count
    return [row(limit=limit, q=f"question {index}", rank=ranks[index]) for index in range(count)]


class Pairing(unittest.TestCase):
    def test_two_runs_that_answered_different_questions_are_refused(self):
        base = questions(3)
        head = base[:-1]
        with self.assertRaises(CannotRun) as raised:
            compare("ratchet", base, head, (10,))
        self.assertIn("do not answer the same questions", str(raised.exception))

    def test_a_question_answered_twice_in_one_run_is_refused(self):
        base = questions(2)
        with self.assertRaises(CannotRun) as raised:
            compare("ratchet", base + [base[0]], questions(2), (10,))
        self.assertIn("twice", str(raised.exception))

    def test_the_same_words_in_two_modes_are_two_questions(self):
        # The corpus asks `fuga de memoria` under the default conjunction and
        # under `any`. They are different questions and the base answers them
        # differently, so the pairing must keep them apart rather than call the
        # second one a duplicate of the first.
        base = [row(mode=None, rank=1), row(mode="any", rank=2)]
        head = [row(mode=None, rank=1), row(mode="any", rank=1)]
        report = compare("ratchet", base, head, (10,))
        self.assertEqual(report["drops"], [])
        self.assertEqual(len(report["moved"]), 1, report["moved"])
        self.assertEqual(report["moved"][0]["mode"], "any")

    def test_a_limit_nobody_measured_is_refused_rather_than_read_as_no_drop(self):
        with self.assertRaises(CannotRun) as raised:
            compare("ratchet", questions(2, limit=10), questions(2, limit=10), (10, 20))
        self.assertIn("limit 20", str(raised.exception))


class Drops(unittest.TestCase):
    def test_a_lost_first_place_fails_both_measures_at_the_default_limit(self):
        base = questions(4, rank=[1, 1, 1, 1])
        head = questions(4, rank=[3, 1, 1, 1])
        report = compare("ratchet", base, head, (10,))
        self.assertEqual(len(report["drops"]), 2, report["drops"])
        self.assertTrue(all("limit 10" in drop for drop in report["drops"]), report["drops"])

    def test_a_fall_below_first_place_fails_mrr_alone(self):
        base = questions(4, rank=[1, 1, 1, 3])
        head = questions(4, rank=[1, 1, 1, 4])
        report = compare("ratchet", base, head, (10,))
        self.assertEqual(len(report["drops"]), 1, report["drops"])
        self.assertIn("MRR at limit 10", report["drops"][0])

    def test_a_gain_passes_and_is_still_shown(self):
        base = questions(4, rank=[4, 1, 1, 1])
        head = questions(4, rank=[1, 1, 1, 1])
        report = compare("ratchet", base, head, (10,))
        self.assertEqual(report["drops"], [])
        self.assertEqual(len(report["moved"]), 1, report["moved"])
        self.assertTrue(rank_worse(4, 1) is False)

    def test_a_fall_at_twenty_is_judged_even_when_ten_did_not_move(self):
        base = questions(2, limit=10) + questions(2, limit=20, rank=1)
        head = questions(2, limit=10) + questions(2, limit=20, rank=[2, 1])
        report = compare("ratchet", base, head, (10, 20))
        self.assertEqual(len(report["drops"]), 2, report["drops"])
        self.assertTrue(all("limit 20" in drop for drop in report["drops"]), report["drops"])

    def test_a_lost_answer_and_a_worse_place_are_both_worse(self):
        self.assertTrue(rank_worse(1, None))
        self.assertFalse(rank_worse(None, 3))
        self.assertFalse(rank_worse(3, 3))
        self.assertTrue(rank_worse(4, 5))

    def test_reply_bytes_are_reported_and_never_a_drop(self):
        base = [row(rank=1, size=400)]
        head = [row(rank=1, size=100)]
        report = compare("ratchet", base, head, (10,))
        self.assertEqual(report["drops"], [])
        self.assertEqual(report["limits"][0]["base_bytes"], 400)
        self.assertEqual(report["limits"][0]["head_bytes"], 100)


class CommandLine(unittest.TestCase):
    def test_limits_are_deduplicated_and_a_zero_is_refused(self):
        self.assertEqual(limits_from("10,20,10"), (10, 20))
        with self.assertRaises(argparse.ArgumentTypeError):
            limits_from("0")
        with self.assertRaises(argparse.ArgumentTypeError):
            limits_from("")

    def test_a_corpus_is_named_at_least_once_and_only_from_the_list(self):
        self.assertEqual(corpora_from("ratchet,ratchet"), ("ratchet",))
        with self.assertRaises(argparse.ArgumentTypeError) as raised:
            corpora_from("")
        self.assertIn("at least one corpus", str(raised.exception))
        with self.assertRaises(argparse.ArgumentTypeError) as raised:
            corpora_from("ratchet,imaginary")
        self.assertIn("imaginary", str(raised.exception))


class Resolution(unittest.TestCase):
    def test_one_query_is_the_smallest_change_a_corpus_can_show(self):
        self.assertEqual(resolution(117), 1 / 117)
        self.assertEqual(resolution(56), 1 / 56)
        self.assertEqual(resolution(0), 0.0)

    def test_the_scored_limits_start_at_the_products_default(self):
        limits = quality.scored_limits()
        self.assertEqual(limits[0], quality.default_limit(), limits)
        self.assertIn(quality.PUBLISHED_LIMIT, limits, limits)

    def test_the_default_is_read_from_the_constant_the_binary_declares(self):
        self.assertGreater(quality.default_limit(), 0)

    def test_a_source_that_no_longer_declares_the_default_says_where(self):
        with tempfile.NamedTemporaryFile("w", suffix=".rs", delete=False) as source:
            source.write("pub(crate) const SOMETHING_ELSE: usize = 10;\n")
        try:
            original = quality.SEARCH_RS
            quality.SEARCH_RS = source.name
            with self.assertRaises(RuntimeError) as raised:
                quality.default_limit()
            self.assertIn("DEFAULT_SEARCH_LIMIT", str(raised.exception))
        finally:
            quality.SEARCH_RS = original
            os.unlink(source.name)

    def test_a_missing_source_names_the_path_it_could_not_read(self):
        original = quality.SEARCH_RS
        quality.SEARCH_RS = os.path.join(tempfile.gettempdir(), "no-such-search.rs")
        try:
            with self.assertRaises(RuntimeError) as raised:
                quality.default_limit()
            self.assertIn("no-such-search.rs", str(raised.exception))
        finally:
            quality.SEARCH_RS = original


class Measured(unittest.TestCase):
    def test_a_question_that_names_a_mode_is_asked_in_it(self):
        self.assertEqual(
            search_arguments({"q": "fuga de memoria", "mode": "any"}, 10),
            {"query": "fuga de memoria", "limit": 10, "match_mode": "any"},
        )

    def test_a_question_without_a_mode_is_asked_as_the_default(self):
        self.assertEqual(
            search_arguments({"q": "fuga de memoria"}, 10),
            {"query": "fuga de memoria", "limit": 10},
        )


@unittest.skipUnless(os.environ.get("LETEO_BIN"), "LETEO_BIN is not set, so the binary under test is not the one on PATH")
class TheDefaultTheBinaryApplies(unittest.TestCase):
    """The parsed number, held to the binary's own behaviour.

    `quality` reads the constant from source, which is a claim about what the
    code says. This is the observation: a search that names no `limit` returns
    as many rows as the harness will ask for when it names the default. A change
    that moves the constant while leaving the applied default behind is caught
    here rather than by a report that measured the wrong configuration.
    """

    def test_a_search_with_no_limit_returns_the_number_the_harness_reads(self):
        from mcpclient import MCP, call_json

        limit = quality.default_limit()
        state = tempfile.mkdtemp(prefix="leteo-default-limit-")
        home = os.path.join(state, "lhome")
        os.makedirs(home)
        server = MCP("leteo", "alpha-api", home=home)
        try:
            for index in range(limit + 3):
                reply, _, _ = call_json(
                    server,
                    "mem_save",
                    {
                        "title": f"Guarded memory {index}",
                        "content": f"the shared word appears in body {index}",
                        "type": "discovery",
                    },
                )
                self.assertEqual(reply.get("status"), "inserted", json.dumps(reply)[:300])
            reply, _, _ = call_json(server, "mem_search", {"query": "shared word"})
        finally:
            server.close()
        self.assertEqual(len(reply.get("results") or []), limit, json.dumps(reply)[:300])


class OneRankForBothInstruments(unittest.TestCase):
    """The ratchet's floors and the paired comparison score a reply alike.

    The two gates are supposed to measure the same thing — one at fixed floors,
    one against the base binary — and they come apart only if the rank rule or
    the project list stops being shared. Importing both is what notices: a copy
    that lands in one of them rebinds a name that was the same object.
    """

    def test_both_read_the_one_rank_and_the_one_project_list(self):
        import mcpclient
        import ratchet
        from precision import PROJECTS as precision_projects
        from precision import rank as precision_rank

        self.assertIs(ratchet.rank, mcpclient.rank)
        self.assertIs(precision_rank, mcpclient.rank)
        self.assertIs(ratchet.PROJECTS, mcpclient.PROJECTS)
        self.assertIs(precision_projects, mcpclient.PROJECTS)

    def test_the_shared_rank_answers_what_each_instrument_used_to_compute(self):
        import ratchet
        from mcpclient import rank

        found = ["first", "second", "third"]
        for position, target in enumerate(found, start=1):
            self.assertEqual(rank(found, target), position)
            self.assertEqual(ratchet.rank(found, target), position)
        self.assertIsNone(rank(found, "somewhere else"))
        self.assertIsNone(rank([], "first"), "an empty answer has no place")


class ExitCodes(unittest.TestCase):
    """The contract the command publishes: 0 clean, 1 a drop, 2 could not run.

    Nothing else calls `main`, so a refactor that let a failed measurement fall
    through to a clean report would leave CI green while measuring nothing.
    """

    def reports(self, rank):
        return {"ratchet": [row(q=f"question {index}", rank=rank) for index in range(4)]}

    def run_main(self, base, head):
        with mock.patch.object(precision, "measure_binary", side_effect=[base, head]):
            with redirect_stdout(io.StringIO()):
                return precision.main(
                    ["--base", __file__, "--head", __file__, "--limits", "10", "--corpora", "ratchet"]
                )

    def test_a_lost_place_exits_one(self):
        self.assertEqual(self.run_main(self.reports(rank=1), self.reports(rank=4)), 1)

    def test_an_unchanged_run_exits_zero(self):
        self.assertEqual(self.run_main(self.reports(rank=2), self.reports(rank=2)), 0)

    def test_a_comparison_that_could_not_run_exits_two(self):
        with mock.patch.object(precision, "measure_binary", side_effect=CannotRun("no binary")):
            with redirect_stdout(io.StringIO()):
                self.assertEqual(
                    precision.main(
                        ["--base", __file__, "--head", __file__, "--limits", "10", "--corpora", "ratchet"]
                    ),
                    2,
                )

    def test_a_child_that_exited_non_zero_is_not_a_clean_run(self):
        finished = subprocess.CompletedProcess(["child"], 1, stdout="", stderr="the child broke")
        with tempfile.TemporaryDirectory() as state:
            with mock.patch.object(precision.subprocess, "run", return_value=finished):
                with self.assertRaises(CannotRun) as raised:
                    precision.measure_binary(__file__, ("ratchet",), (10,), state)
        self.assertIn("exit 1", str(raised.exception))

    def test_a_child_that_wrote_no_ranks_is_not_a_clean_run(self):
        finished = subprocess.CompletedProcess(["child"], 0, stdout="", stderr="")
        with tempfile.TemporaryDirectory() as state:
            with mock.patch.object(precision.subprocess, "run", return_value=finished):
                with self.assertRaises(CannotRun) as raised:
                    precision.measure_binary(__file__, ("ratchet",), (10,), state)
        self.assertIn("wrote no readable ranks", str(raised.exception))


if __name__ == "__main__":
    unittest.main()
