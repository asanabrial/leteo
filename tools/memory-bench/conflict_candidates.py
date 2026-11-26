#!/usr/bin/env python3
"""What does a binary propose for judgment, and how much of it is a real conflict?

    python3 tools/memory-bench/conflict_candidates.py --base /tmp/leteo-base --head target/release/leteo

Each binary loads `conflict_corpus` into a store of its own through the MCP
protocol and every save's reply is scored against the corpus's label: a
proposal is a true one when it names the memory the saver is labelled as
contradicting, and a false one otherwise. Precision is the true share of every
proposal; recall is how many labelled contradictions were proposed at all.
Both are printed per language and together, beside how many saves asked for a
judgment — the number the cost of the feature is made of.

A model is not needed and does not take part: the candidate finder is lexical.
When `LETEO_MODEL_DIR` is set it is honoured, so a run says which configuration
it measured rather than leaving a reader to assume.

The two runs are separate child processes because `mcpclient` resolves
`LETEO_BIN` when it is imported, and no process can import it twice. It exits 2
when it could not measure at all and 0 otherwise: this is an instrument, not a
gate, because a corpus of this size cannot prove a floor.
"""

import argparse
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import conflict_corpus  # noqa: E402  (the path above has to be set first)
from mcpclient import MCP, call_json  # noqa: E402

LANGUAGES = ("en", "es")


class CannotRun(Exception):
    pass


# --- measuring (child process) ------------------------------------------------


def measure():
    """Save the corpus into a fresh store and report what each save proposed.

    A save that is not `inserted` stops the run instead of being scored. A
    deduplicated or revised save has no proposal of its own and the corpus
    memory it was labelled with is not in the store as a separate row, so
    counting it would silently score a store the corpus does not describe.
    """
    state = os.environ["BENCH_STATE"]
    home = os.path.join(state, "lhome")
    os.makedirs(home, exist_ok=True)
    server = MCP("leteo", conflict_corpus.PROJECT, home=home)
    keys = {}
    rows = []
    try:
        for memory in conflict_corpus.memories():
            reply, _, _ = call_json(
                server,
                "mem_save",
                {
                    "title": memory["title"],
                    "content": memory["content"],
                    "type": "discovery",
                },
            )
            if reply.get("status") != "inserted":
                raise CannotRun(
                    f"{memory['key']} came back {reply.get('status')!r}, not inserted; "
                    "the corpus needs every save to be its own memory"
                )
            observation = reply.get("observation") or {}
            row_id = observation.get("id")
            if row_id is None:
                raise CannotRun(f"{memory['key']} was inserted without an id")
            if row_id in keys:
                raise CannotRun(f"{memory['key']} and {keys[row_id]} were given one id")
            keys[row_id] = memory["key"]
            if memory["phase"] != "probe":
                continue
            proposed = []
            for candidate in reply.get("candidates") or []:
                key = keys.get(candidate.get("id"))
                if key is None:
                    raise CannotRun(
                        f"{memory['key']} proposed a candidate that is not a corpus memory: "
                        f"{candidate.get('id')}"
                    )
                proposed.append(key)
            rows.append(
                dict(
                    key=memory["key"],
                    lang=memory["lang"],
                    contradicts=memory["contradicts"],
                    proposed=proposed,
                    judgment_required=bool(reply.get("judgment_required")),
                )
            )
    finally:
        server.close()
    return rows


# --- scoring ------------------------------------------------------------------


def score(rows):
    """The counts precision and recall are made of, for a set of saved probes."""
    proposals = [key for row in rows for key in row["proposed"]]
    true = [
        key
        for row in rows
        for key in row["proposed"]
        if row["contradicts"] is not None and key == row["contradicts"]
    ]
    labelled = [row for row in rows if row["contradicts"] is not None]
    unrelated = [row for row in rows if row["contradicts"] is None]
    return dict(
        saves=len(rows),
        asked=sum(1 for row in rows if row["judgment_required"]),
        proposals=len(proposals),
        true_proposals=len(true),
        labelled=len(labelled),
        found=sum(1 for row in labelled if row["contradicts"] in row["proposed"]),
        unrelated=len(unrelated),
        unrelated_asked=sum(1 for row in unrelated if row["proposed"]),
    )


def precision(counts):
    """The true share of the proposals, or `None` when there were none.

    `None` rather than zero: a binary that proposes nothing did not achieve
    perfect precision, it declined to answer, and the recall beside it is where
    that shows.
    """
    return counts["true_proposals"] / counts["proposals"] if counts["proposals"] else None


def recall(counts):
    return counts["found"] / counts["labelled"] if counts["labelled"] else None


def by_language(rows):
    return {language: score([row for row in rows if row["lang"] == language]) for language in LANGUAGES}


def percent(value):
    return "-" if value is None else f"{value * 100:.0f}%"


def print_score(label, counts):
    print(
        f"  {label:<4} saves {counts['saves']:>2}  asked {counts['asked']:>2}  "
        f"proposals {counts['proposals']:>2}  precision {percent(precision(counts)):>4}  "
        f"contradictions {counts['found']}/{counts['labelled']}  "
        f"recall {percent(recall(counts)):>4}  "
        f"unrelated saves asking {counts['unrelated_asked']}/{counts['unrelated']}"
    )


def print_report(label, rows, detail):
    print(f"{label}: {len(rows)} saves scored, model "
          f"{'present' if os.environ.get('LETEO_MODEL_DIR') else 'absent'}")
    print_score("all", score(rows))
    languages = by_language(rows)
    for language in LANGUAGES:
        print_score(language, languages[language])
    if not detail:
        return
    for row in rows:
        if row["proposed"]:
            expected = row["contradicts"] or "-"
            print(f"    {row['key']:<20} proposed {', '.join(row['proposed']):<45} expected {expected}")


# --- the command --------------------------------------------------------------


def measure_binary(binary, state):
    """One binary's rows, from a child process with its own store."""
    path = os.path.join(state, "rows.json")
    os.makedirs(state, exist_ok=True)
    environment = dict(os.environ)
    environment.update(
        BENCH_STATE=os.path.join(state, "bench"),
        LETEO_BIN=os.path.abspath(binary),
    )
    command = [sys.executable, os.path.abspath(__file__), "--measure", "--out", path]
    finished = subprocess.run(command, env=environment, capture_output=True, text=True)
    if finished.returncode != 0:
        detail = (finished.stderr or finished.stdout).strip().splitlines()
        raise CannotRun(
            f"measuring {binary} failed with exit {finished.returncode}: "
            f"{detail[-1] if detail else 'no output'}"
        )
    try:
        with open(path, encoding="utf-8") as rows:
            return json.load(rows)["rows"]
    except (OSError, KeyError, ValueError) as error:
        raise CannotRun(f"measuring {binary} wrote no readable rows: {error}")


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--base", help="the binary the change is measured against")
    parser.add_argument("--head", help="the binary under test")
    parser.add_argument("--detail", action="store_true", help="print every save that proposed something")
    parser.add_argument("--measure", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--out", help=argparse.SUPPRESS)
    args = parser.parse_args(argv)

    if args.measure:
        if not args.out:
            parser.error("--measure needs --out")
        try:
            rows = measure()
        except (CannotRun, OSError, RuntimeError, KeyError, ValueError) as error:
            print(f"conflict candidates could not measure: {type(error).__name__}: {error}", file=sys.stderr)
            return 2
        with open(args.out, "w", encoding="utf-8") as out:
            json.dump({"rows": rows}, out, ensure_ascii=False)
        return 0

    if not args.base or not args.head:
        parser.error("both --base and --head are needed")
    for binary in (args.base, args.head):
        if not os.path.isfile(binary):
            parser.error(f"{binary} is not a file; name the binary to measure")

    try:
        with tempfile.TemporaryDirectory(prefix="leteo-conflict-") as state:
            base = measure_binary(args.base, os.path.join(state, "base"))
            head = measure_binary(args.head, os.path.join(state, "head"))
    except (CannotRun, OSError, RuntimeError, KeyError, ValueError) as error:
        print(f"conflict candidates could not run: {type(error).__name__}: {error}")
        return 2

    print_report(f"base {args.base}", base, args.detail)
    print()
    print_report(f"head {args.head}", head, args.detail)
    print()
    print_score("base", score(base))
    print_score("head", score(head))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
