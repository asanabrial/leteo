#!/usr/bin/env python3
"""Does this build find what the last one found?

    python3 tools/memory-bench/precision.py --base target/base/leteo --head target/release/leteo

Both binaries load the same corpus into a store of their own and are asked the
same questions at the limit an agent gets and at the harness's own twenty. The
answer is paired per question — the rank it came back at in one run beside the
rank in the other — so a query that fell from first to fourth is shown even
when another rose and the means agree.

It exits 1 when hit@1 or MRR is lower on the second binary at any scored limit,
2 when it could not run at all, and 0 when nothing fell. Reply bytes are
reported and not judged: a smaller reply is the point of most of the changes
this is meant to guard, and the byte ceilings in `floors.json` already bound it.

Two corpora are measured, and each says what one query is worth in it:

- `corpus.py`, the one the ratchet gates on: 117 queries, so one query moves
  hit@1 by 0.86 points.
- `comparison_corpus.py`, the held-out corpus the Engram comparison uses: 56
  queries, so one query moves hit@1 by 1.79 points.

A difference smaller than the resolution of a corpus is not visible in it, and
printing both numbers is what keeps a reader from reading 0.9 points on 117
queries as if it were the same evidence as 0.9 on 56.

Each binary is measured in a child process: `mcpclient` resolves `LETEO_BIN`
and the state directory when it is imported, and no single process can import
it twice. The child writes its ranks to JSON and says nothing on stdout, so the
report a person reads is built only from what was measured.
"""

import argparse
import importlib
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import quality  # noqa: E402  (the path above has to be set first)
from mcpclient import MCP, PROJECTS, call_json, rank, save_corpus  # noqa: E402
from scoring import summarise  # noqa: E402

# The corpus each name stands for. The names are what the output and the issue
# call them; the modules are where the memories and questions live.
CORPORA = {"ratchet": "corpus", "comparison": "comparison_corpus"}


class CannotRun(Exception):
    pass


# --- measuring (child process) ----------------------------------------------


def search_arguments(question, limit):
    """What is sent for one question at one limit.

    The mode is part of the question, not a property of the store: the corpus
    asks the same words under the default conjunction and under `any`, and a
    request that dropped the mode would answer the default one twice.
    """
    args = {"query": question["q"], "limit": limit}
    if question.get("mode"):
        args["match_mode"] = question["mode"]
    return args


def measure(corpora, limits):
    """Load each corpus into a store of its own and rank every question.

    A store per corpus rather than one per binary: both corpora use the same two
    project names, so one store would answer the second corpus's questions with
    the first corpus's memories.
    """
    state = os.environ["BENCH_STATE"]
    measured = {}
    for name in corpora:
        module = importlib.import_module(CORPORA[name])
        members, wanted = module.corpus(), module.queries()
        named = sorted({memory["project"] for memory in members} | {q["project"] for q in wanted})
        if named != sorted(PROJECTS):
            raise CannotRun(
                f"the {name} corpus names the projects {named}; this harness starts {list(PROJECTS)}"
            )
        home = os.path.join(state, name, "lhome")
        os.makedirs(home, exist_ok=True)
        # Every corpus here carries Spanish memories and Spanish questions, and
        # the language a memory is stemmed in is the store's setting: a store
        # that does not name it indexes them as English and the stemmer under
        # test never runs.
        with open(os.path.join(home, "settings.json"), "w") as settings:
            json.dump({"language": "Spanish"}, settings)
        servers = {project: MCP("leteo", project, home=home) for project in PROJECTS}
        try:
            ids = save_corpus(servers, members)

            rows = []
            for limit in limits:
                for question in wanted:
                    reply, text, _ = call_json(
                        servers[question["project"]],
                        "mem_search",
                        search_arguments(question, limit),
                    )
                    found = [r["id"] for r in reply.get("results") or []]
                    rows.append(
                        dict(
                            kind=question["kind"],
                            q=question["q"],
                            project=question["project"],
                            limit=limit,
                            mode=question.get("mode"),
                            rank=rank(found, ids[question["target"]]),
                            results=len(found),
                            bytes=len(text.encode()),
                        )
                    )
        finally:
            for server in servers.values():
                try:
                    server.close()
                except Exception:  # noqa: BLE001 - a child that cannot close is measured no further
                    server.p.kill()
        measured[name] = rows
    return measured


# --- comparing (parent) -----------------------------------------------------


def identity(row):
    """What makes two rows the same question in two runs.

    The mode belongs in it: `fuga de memoria` under `any` and under the default
    conjunction are two different questions to the store, and only the mode
    tells the corpus entries apart.
    """
    return (row["limit"], row["kind"], row["project"], row["q"], row["mode"] or "")


def paired(base, head):
    """The two runs' rows, one pair per question and limit.

    Refuses a run that answered a different set of questions rather than pairing
    what overlaps: a corpus that grew on one side would otherwise compare like a
    regression, and one that shrank would hide one.
    """
    left, right = {}, {}
    for side, rows, into in (("base", base, left), ("head", head, right)):
        for row in rows:
            key = identity(row)
            if key in into:
                raise CannotRun(f"the {side} run answered {key!r} twice")
            into[key] = row
    if set(left) != set(right):
        only_base = sorted(set(left) - set(right))
        only_head = sorted(set(right) - set(left))
        raise CannotRun(
            f"the two runs do not answer the same questions; base only: {only_base[:3]}, "
            f"head only: {only_head[:3]}"
        )
    return [(left[key], right[key]) for key in sorted(left)]


def resolution(queries):
    """What one query is worth in hit@1 on a corpus of this many questions."""
    return 1.0 / queries if queries else 0.0


def compare(name, base, head, limits):
    """One corpus's report, and the drops that make the command fail."""
    pairs = paired(base, head)
    report = {"corpus": name, "queries": 0, "limits": [], "moved": [], "drops": []}
    for limit in limits:
        at = [(b, h) for b, h in pairs if b["limit"] == limit]
        if not at:
            raise CannotRun(f"no question was measured at limit {limit} for {name}")
        before = summarise([b for b, _ in at])
        after = summarise([h for _, h in at])
        report["queries"] = before["n"]
        report["limits"].append(
            dict(
                limit=limit,
                base=before,
                head=after,
                base_bytes=statistics.mean(b["bytes"] for b, _ in at),
                head_bytes=statistics.mean(h["bytes"] for _, h in at),
            )
        )
        for b, h in at:
            if b["rank"] != h["rank"]:
                report["moved"].append(
                    dict(
                        limit=limit,
                        kind=b["kind"],
                        q=b["q"],
                        project=b["project"],
                        mode=b["mode"],
                        base=b["rank"],
                        head=h["rank"],
                    )
                )
        if after["hit1"] < before["hit1"]:
            report["drops"].append(
                f"hit@1 at limit {limit}: {before['hit1']:.3f} -> {after['hit1']:.3f}"
            )
        if after["mrr"] < before["mrr"]:
            report["drops"].append(
                f"MRR at limit {limit}: {before['mrr']:.3f} -> {after['mrr']:.3f}"
            )
    return report


def place(rank):
    return "-" if rank is None else str(rank)


def print_report(report):
    queries = report["queries"]
    print(
        f"{report['corpus']} corpus: {queries} queries "
        f"(smallest hit@1 change it can show: 1/{queries} = {resolution(queries) * 100:.2f} points)"
    )
    for line in report["limits"]:
        before, after = line["base"], line["head"]
        print(
            f"  limit {line['limit']:>2}  hit@1 {before['hit1']:.3f} -> {after['hit1']:.3f}  "
            f"MRR {before['mrr']:.3f} -> {after['mrr']:.3f}  "
            f"mean reply {line['base_bytes']:.0f} -> {line['head_bytes']:.0f} bytes"
        )
    for moved in report["moved"]:
        direction = "lost" if rank_worse(moved["base"], moved["head"]) else "gained"
        mode = f"[{moved['mode']}] " if moved["mode"] else ""
        print(
            f"  {direction:6} limit {moved['limit']:>2} {moved['kind']:<10} "
            f"{moved['project']:<9} rank {place(moved['base'])} -> {place(moved['head'])}  "
            f"{mode}{moved['q']!r}"
        )
    for drop in report["drops"]:
        print(f"DROPPED {drop}")


def rank_worse(before, after):
    """Whether the second rank is the worse one, with absence worst of all."""
    if after is None:
        return before is not None
    if before is None:
        return False
    return after > before


# --- the command ------------------------------------------------------------


def measure_binary(binary, corpora, limits, state):
    """One binary's ranks, from a child process with its own store."""
    path = os.path.join(state, "ranks.json")
    os.makedirs(state, exist_ok=True)
    environment = dict(os.environ)
    environment.update(
        BENCH_STATE=os.path.join(state, "bench"),
        LETEO_BIN=os.path.abspath(binary),
    )
    command = [
        sys.executable,
        os.path.abspath(__file__),
        "--measure",
        "--out",
        path,
        "--corpora",
        ",".join(corpora),
        "--limits",
        ",".join(str(limit) for limit in limits),
    ]
    finished = subprocess.run(command, env=environment, capture_output=True, text=True)
    if finished.returncode != 0:
        detail = (finished.stderr or finished.stdout).strip().splitlines()
        raise CannotRun(
            f"measuring {binary} failed with exit {finished.returncode}: "
            f"{detail[-1] if detail else 'no output'}"
        )
    try:
        with open(path, encoding="utf-8") as ranks:
            return json.load(ranks)["corpora"]
    except (OSError, KeyError, ValueError) as error:
        raise CannotRun(f"measuring {binary} wrote no readable ranks: {error}")


def limits_from(text):
    try:
        limits = tuple(dict.fromkeys(int(part) for part in text.split(",") if part.strip()))
    except ValueError:
        raise argparse.ArgumentTypeError(f"{text!r} is not a comma-separated list of row counts")
    if not limits or any(limit < 1 for limit in limits):
        raise argparse.ArgumentTypeError("every limit must be a positive row count")
    return limits


def corpora_from(text):
    names = tuple(dict.fromkeys(part.strip() for part in text.split(",") if part.strip()))
    if not names:
        raise argparse.ArgumentTypeError(f"name at least one corpus: {', '.join(CORPORA)}")
    unknown = [name for name in names if name not in CORPORA]
    if unknown:
        raise argparse.ArgumentTypeError(
            f"unknown corpus {unknown[0]!r}; name one of {', '.join(CORPORA)}"
        )
    return names


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--base", help="the binary the change is measured against")
    parser.add_argument("--head", help="the binary under test")
    parser.add_argument(
        "--limits",
        default=None,
        help="comma-separated row counts; defaults to the product's default and 20",
    )
    parser.add_argument("--corpora", default=",".join(CORPORA), help="comma-separated corpus names")
    parser.add_argument("--measure", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--out", help=argparse.SUPPRESS)
    args = parser.parse_args(argv)

    try:
        limits = limits_from(args.limits) if args.limits else quality.scored_limits()
        corpora = corpora_from(args.corpora)
    except (argparse.ArgumentTypeError, RuntimeError) as error:
        parser.error(str(error))

    if args.measure:
        if not args.out:
            parser.error("--measure needs --out")
        try:
            rows = measure(corpora, limits)
        except (CannotRun, OSError, RuntimeError, KeyError, ValueError) as error:
            print(f"precision could not measure: {type(error).__name__}: {error}", file=sys.stderr)
            return 2
        with open(args.out, "w", encoding="utf-8") as out:
            json.dump({"corpora": rows}, out, ensure_ascii=False)
        return 0

    if not args.base or not args.head:
        parser.error("both --base and --head are needed")
    for binary in (args.base, args.head):
        if not os.path.isfile(binary):
            parser.error(f"{binary} is not a file; name the binary to measure")

    state = tempfile.mkdtemp(prefix="leteo-precision-")
    try:
        base = measure_binary(args.base, corpora, limits, os.path.join(state, "base"))
        head = measure_binary(args.head, corpora, limits, os.path.join(state, "head"))
        reports = [compare(name, base[name], head[name], limits) for name in corpora]
    except (CannotRun, OSError, RuntimeError, KeyError, ValueError) as error:
        print(f"precision could not run: {type(error).__name__}: {error}")
        return 2
    finally:
        shutil.rmtree(state, ignore_errors=True)

    print(f"base {args.base}\nhead {args.head}")
    for report in reports:
        print()
        print_report(report)
    drops = [drop for report in reports for drop in report["drops"]]
    if drops:
        print(f"\nprecision FAILED: {len(drops)} drop(s) across the corpora compared")
        return 1
    print("\nprecision passed: nothing fell")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
