"""Self-checks for precision_gate: the pairing and the rule that fails a drop.

    python3 -m unittest discover -s tools/memory-bench -p 'test_precision_gate.py'

No binary is started. `measure` is the only function that needs one, and every
test here feeds `compare` a fixed list of rows.
"""
import unittest

import precision_gate


def row(kind="k", question="question", target="t", rank=None, size=100):
    return dict(kind=kind, q=question, target=target, project="alpha-api", rank=rank, results=10, bytes=size)


class Compare(unittest.TestCase):
    def test_an_identical_run_passes(self):
        rows = [row(question="a", rank=1), row(question="b", rank=2)]
        result = precision_gate.compare(rows, [dict(r) for r in rows])
        self.assertEqual(result["breaches"], [])
        self.assertEqual(result["rows"]["ALL"]["base"]["hit1"], 0.5)
        self.assertEqual(result["rows"]["ALL"]["base"]["mrr"], 0.75)

    def test_losing_first_place_fails_on_hit1(self):
        base = [row(question="a", rank=1), row(question="b", rank=1)]
        head = [row(question="a", rank=1), row(question="b", rank=2)]
        result = precision_gate.compare(base, head)
        self.assertTrue(
            any(line.startswith("hit1 ALL") for line in result["breaches"]),
            result["breaches"],
        )

    def test_a_lower_average_fails_on_mrr_even_when_nothing_left_the_first_rows(self):
        # Both questions are still inside the default ten, so hit@1 says
        # nothing about the change and MRR has to carry it.
        base = [row(question="a", rank=2), row(question="b", rank=2)]
        head = [row(question="a", rank=3), row(question="b", rank=2)]
        result = precision_gate.compare(base, head)
        self.assertTrue(
            any(line.startswith("mrr ALL") for line in result["breaches"]),
            result["breaches"],
        )

    def test_a_swap_is_reported_per_question_although_the_average_is_unchanged(self):
        # One question lost first place and another gained it: MRR and hit@1 are
        # both identical, so only the paired list can show what happened.
        base = [row(question="a", rank=1), row(question="b", rank=2)]
        head = [row(question="a", rank=2), row(question="b", rank=1)]
        result = precision_gate.compare(base, head)
        self.assertEqual(result["breaches"], [])
        moved = {pair["key"][1]: (pair["base"], pair["head"]) for pair in result["paired"]}
        self.assertEqual(moved, {"a": (1, 2), "b": (2, 1)})

    def test_a_drop_in_one_kind_is_shown_but_does_not_fail_when_the_total_rises(self):
        # The acceptance gates the aggregate; the per-kind table and the paired
        # list are what show the trade to whoever decides.
        base = [row(kind="a", question="a", rank=1), row(kind="b", question="b", rank=3)]
        head = [row(kind="a", question="a", rank=2), row(kind="b", question="b", rank=1)]
        result = precision_gate.compare(base, head)
        self.assertEqual(result["breaches"], [])
        self.assertEqual(result["rows"]["a"]["base"]["hit1"], 1.0)
        self.assertEqual(result["rows"]["a"]["head"]["hit1"], 0.0)

    def test_a_question_answered_on_one_side_only_is_refused(self):
        with self.assertRaises(precision_gate.CannotRun):
            precision_gate.compare([row(question="a", rank=1)], [row(question="b", rank=1)])

    def test_reply_bytes_are_reported_beside_the_ranks(self):
        base = [row(question="a", rank=1, size=1000), row(question="b", rank=1, size=2000)]
        head = [row(question="a", rank=1, size=500), row(question="b", rank=1, size=500)]
        result = precision_gate.compare(base, head)
        self.assertEqual(result["rows"]["ALL"]["head"]["bytes"], 500)
        self.assertEqual(result["breaches"], [])


class Report(unittest.TestCase):
    def test_the_failed_verdict_says_the_second_binary_is_worse(self):
        base = [row(question="a", rank=1)]
        head = [row(question="a", rank=None)]
        self.assertEqual(precision_gate.report(precision_gate.compare(base, head)), 1)

    def test_a_pass_reports_zero(self):
        rows = [row(question="a", rank=1)]
        self.assertEqual(precision_gate.report(precision_gate.compare(rows, rows)), 0)


if __name__ == "__main__":
    unittest.main()
