"""Self-checks for the conflict candidates instrument: its arithmetic and its corpus.

    python3 -m unittest discover -s tools/memory-bench -p 'test_*.py'

No binary is started. Every scoring case builds the rows `measure` would have
written by hand, so the arithmetic is checked without a store; the corpus cases
check the labelled set's shape, because a corpus that lost half its negatives or
its same-title pair would still score and would quietly measure something else.
"""
import os
import tempfile
import unittest

os.environ.setdefault(
    "BENCH_STATE", os.path.join(tempfile.gettempdir(), "leteo-conflict-selfcheck")
)

import conflict_corpus  # noqa: E402
import conflict_candidates as cc  # noqa: E402


def row(key="probe", lang="en", contradicts=None, proposed=(), asked=None):
    return dict(
        key=key,
        lang=lang,
        contradicts=contradicts,
        proposed=list(proposed),
        judgment_required=(bool(proposed) if asked is None else asked),
    )


class Arithmetic(unittest.TestCase):
    def test_a_proposal_of_the_labelled_memory_is_the_only_true_one(self):
        rows = [
            row("a", contradicts="seed-a", proposed=["seed-a"]),
            row("b", contradicts="seed-b", proposed=["seed-a"]),
            row("c", proposed=["seed-a", "seed-b"]),
        ]
        counts = cc.score(rows)
        self.assertEqual(counts["proposals"], 4)
        self.assertEqual(counts["true_proposals"], 1)
        self.assertEqual(counts["found"], 1)
        self.assertEqual(counts["labelled"], 2)
        self.assertEqual(cc.precision(counts), 1 / 4)
        self.assertEqual(cc.recall(counts), 1 / 2)

    def test_saves_that_asked_are_counted_apart_from_the_proposals(self):
        rows = [
            row("a", contradicts="seed-a", proposed=["seed-a"], asked=True),
            row("b", proposed=[], asked=False),
            row("c", proposed=["seed-a"], asked=True),
        ]
        counts = cc.score(rows)
        self.assertEqual(counts["asked"], 2)
        self.assertEqual(counts["unrelated"], 2)
        self.assertEqual(counts["unrelated_asked"], 1)

    def test_no_proposal_is_no_precision_rather_than_perfect_precision(self):
        counts = cc.score([row("a", contradicts="seed-a")])
        self.assertIsNone(cc.precision(counts))
        self.assertEqual(cc.recall(counts), 0.0)
        self.assertEqual(cc.percent(cc.precision(counts)), "-")

    def test_a_corpus_without_labels_has_no_recall_to_report(self):
        counts = cc.score([row("a", proposed=["seed-a"])])
        self.assertIsNone(cc.recall(counts))
        self.assertEqual(cc.precision(counts), 0.0)

    def test_the_languages_are_scored_apart(self):
        rows = [
            row("a", lang="en", contradicts="seed-a", proposed=["seed-a"]),
            row("b", lang="en", proposed=["seed-a"]),
            row("c", lang="es", contradicts="seed-b", proposed=["seed-b"]),
        ]
        languages = cc.by_language(rows)
        self.assertEqual(languages["en"]["proposals"], 2)
        self.assertEqual(languages["en"]["true_proposals"], 1)
        self.assertEqual(languages["es"]["proposals"], 1)
        self.assertEqual(cc.precision(languages["es"]), 1.0)
        self.assertEqual(cc.precision(languages["en"]), 0.5)


class Corpus(unittest.TestCase):
    def test_every_save_has_its_own_key_and_each_contradiction_names_a_seed(self):
        memories = conflict_corpus.memories()
        keys = [memory["key"] for memory in memories]
        self.assertEqual(len(keys), len(set(keys)), "two corpus memories share a key")
        seeds = {memory["key"] for memory in memories if memory["phase"] == "seed"}
        self.assertEqual(len(seeds), len(conflict_corpus.SEEDS))
        for memory in memories:
            if memory["contradicts"] is not None:
                self.assertIn(memory["contradicts"], seeds, memory["key"])
                self.assertEqual(memory["phase"], "probe", memory["key"])

    def test_the_labelled_set_keeps_its_two_halves(self):
        probes = conflict_corpus.PROBES
        contradictions = [probe for probe in probes if probe["contradicts"] is not None]
        unrelated = [probe for probe in probes if probe["contradicts"] is None]
        self.assertGreaterEqual(len(contradictions), 12, "the recall half shrank")
        self.assertGreaterEqual(len(unrelated), 24, "the precision half shrank")
        for language in ("en", "es"):
            self.assertEqual(
                sum(1 for probe in contradictions if probe["lang"] == language) * 2,
                len(contradictions),
                f"the contradictions are not half {language}",
            )
            self.assertEqual(
                sum(1 for probe in unrelated if probe["lang"] == language) * 2,
                len(unrelated),
                f"the unrelated saves are not half {language}",
            )

    def test_a_same_title_contradiction_is_in_the_set(self):
        same_title = conflict_corpus.same_title_pairs()
        self.assertTrue(same_title, "the case the issue found missed is gone")
        labelled = {probe["key"]: probe for probe in conflict_corpus.PROBES}
        for key in same_title:
            self.assertIsNotNone(labelled[key]["contradicts"], f"{key} is same-title but unlabelled")


if __name__ == "__main__":
    unittest.main()
