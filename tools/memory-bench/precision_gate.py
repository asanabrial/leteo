#!/usr/bin/env python3
"""Fail when a build finds fewer of the same memories than the build it replaces.

The ratchet compares one binary against fixed floors. That says nothing about a
change that trades one question for another: a loss on a kind whose floor has
room passes, and the aggregate hides which question moved. This runs the same
117-question corpus through two binaries, writes it into two isolated stores,
and asks every question of both at the limit an agent gets when it names none,
so a change to the default is measured rather than assumed away.

What is reported per question is the pair, base rank and head rank, because a
drop on one question and a gain on another is exactly what an aggregate cannot
show and exactly what someone reviewing a ranking change needs to see. Fifty-six
of the comparison corpus's questions would put one question at 1.79 points of
hit@1; here one question is 0.85 points, which is the smallest change this
corpus can detect.

    python3 tools/memory-bench/precision_gate.py --base <base-binary> --head <head-binary>

CI builds the merge base in a worktree and the change in the checkout, and runs
this between them. Ranking transfers between SQLite builds (timing does not), so
the answer means the same thing on a laptop and on a runner.
"""

import argparse
import json
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
# The corpus and the client live beside this file but import by bare name, the
# way every other script in the harness starts them.
sys.path.insert(0, HERE)

PROJECTS = ("alpha-api", "beta-web")


class CannotRun(Exception):
    pass


def settings_for(home):
    # The store's language is its setting, and the corpus's Spanish half is
    # scoring the Spanish stemmer: without this every memory is indexed as
    # English and the half is measured without the stage it exists to measure.
    os.makedirs(home, exist_ok=True)
    with open(os.path.join(home, "settings.json"), "w") as handle:
        json.dump({"language": "Spanish"}, handle)


def measure(binary, state, members, wanted):
    """Every question asked of one binary, in its own store.

    The reply is read through `call_json`, so an error or unparsable answer
    fails the run instead of being scored as a question the binary could not
    answer. `limit` is deliberately not sent: the default is what an agent
    gets, and a change to it is one of the things this compares.
    """
    from mcpclient import MCP, call_json, engine_cmd

    home = os.path.join(state, "lhome")
    settings_for(home)
    servers = {
        project: MCP("leteo", project, launch=engine_cmd("leteo", project, home=home, binary=binary))
        for project in PROJECTS
    }
    try:
        ids = {}
        for mem in members:
            args = dict(title=mem["title"], content=mem["content"], type=mem["type"])
            if mem["topic"]:
                args["topic_key"] = mem["topic"]
            reply, _, _ = call_json(servers[mem["project"]], "mem_save", args)
            if reply.get("status") != "inserted" or reply.get("project") != mem["project"]:
                raise CannotRun(
                    f"{binary}: {mem['key']} was not inserted into {mem['project']}: "
                    f"{json.dumps(reply)[:300]}"
                )
            ids[mem["key"]] = reply["observation"]["id"]
        if len(set(ids.values())) != len(members):
            raise CannotRun(f"{binary}: two corpus memories were given the same id")

        rows = []
        for question in wanted:
            reply, text, _ = call_json(
                servers[question["project"]], "mem_search", {"query": question["q"]}
            )
            found = [result["id"] for result in reply.get("results") or []]
            target = ids[question["target"]]
            rows.append(
                dict(
                    kind=question["kind"],
                    q=question["q"],
                    target=question["target"],
                    project=question["project"],
                    rank=found.index(target) + 1 if target in found else None,
                    results=len(found),
                    bytes=len(text.encode()),
                )
            )
        if len(rows) != len(wanted) or not wanted:
            raise CannotRun(f"{binary}: {len(rows)} questions were asked of {len(wanted)}")
        return rows
    finally:
        for server in servers.values():
            try:
                server.close()
            except Exception:
                server.p.kill()


def keyed(rows):
    """Questions are identified by the question, not by their position.

    Both runs ask the same list, but a pairing that trusts the order is one
    reordered list away from scoring one question's rank against another's.
    """
    return {(row["kind"], row["q"], row["target"]): row for row in rows}


def summarise(rows):
    n = len(rows)
    return dict(
        n=n,
        hit1=sum(row["rank"] == 1 for row in rows) / n,
        mrr=sum(1 / row["rank"] for row in rows if row["rank"]) / n,
        bytes=sum(row["bytes"] for row in rows) / n,
    )


def compare(base_rows, head_rows):
    """The paired difference, and every question whose answer moved.

    A question present on one side only is a corpus mismatch and not a ranking
    result, so it is refused rather than scored as a zero.
    """
    base, head = keyed(base_rows), keyed(head_rows)
    if base.keys() != head.keys():
        missing = sorted(set(base) ^ set(head))
        raise CannotRun(f"the two runs answered different questions: {missing[:5]}")
    order = list(base)
    paired = [
        dict(
            key=key,
            base=base[key]["rank"],
            head=head[key]["rank"],
            base_bytes=base[key]["bytes"],
            head_bytes=head[key]["bytes"],
        )
        for key in order
    ]
    kinds = sorted({key[0] for key in order})
    rows = dict(
        ALL=dict(base=summarise(base_rows), head=summarise(head_rows)),
        **{
            kind: dict(
                base=summarise([row for row in base_rows if row["kind"] == kind]),
                head=summarise([row for row in head_rows if row["kind"] == kind]),
            )
            for kind in kinds
        },
    )
    breaches = []
    # The acceptance is about the aggregate an agent experiences: one question
    # traded for another is a decision for whoever reads the per-kind table and
    # the paired list above, not something a gate can call. A drop in the
    # overall figures is what must fail.
    for metric in ("hit1", "mrr"):
        base, head = rows["ALL"]["base"][metric], rows["ALL"]["head"][metric]
        if head < base:
            breaches.append(f"{metric} ALL: base {base:.3f} -> head {head:.3f}")
    return dict(rows=rows, paired=paired, breaches=breaches)


def report(result):
    print(f"{'kind':12} {'n':>3} {'base h@1':>9} {'head h@1':>9} {'base MRR':>9} {'head MRR':>9} "
          f"{'base bytes':>10} {'head bytes':>10}")
    for name, figures in result["rows"].items():
        base, head = figures["base"], figures["head"]
        print(f"{name:12} {base['n']:>3} {base['hit1']:>9.3f} {head['hit1']:>9.3f} "
              f"{base['mrr']:>9.3f} {head['mrr']:>9.3f} {base['bytes']:>10.0f} {head['bytes']:>10.0f}")
    moved = [p for p in result["paired"] if p["base"] != p["head"]]
    for pair in moved:
        print(f"  {pair['key'][0]:10} {pair['base']!s:>4} -> {pair['head']!s:<4} "
              f"{pair['base_bytes']:>6} -> {pair['head_bytes']:<6} bytes  {pair['key'][1]}")
    print(f"{len(moved)} of {len(result['paired'])} questions changed rank")
    for line in result["breaches"]:
        print(f"BREACH {line}")
    if result["breaches"]:
        print("precision gate FAILED: the second binary ranks worse")
        return 1
    print("precision gate passed")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", required=True, help="binary built from the merge base")
    parser.add_argument("--head", required=True, help="binary built from the change")
    args = parser.parse_args(argv)

    # Set before the harness is imported, because the client reads it at import
    # and a run that reused a store would measure the previous run's rows.
    state = tempfile.mkdtemp(prefix="leteo-precision-")
    os.environ["BENCH_STATE"] = state
    try:
        from corpus import corpus, queries

        members, wanted = corpus(), queries()
        base = measure(args.base, os.path.join(state, "base"), members, wanted)
        head = measure(args.head, os.path.join(state, "head"), members, wanted)
        return report(compare(base, head))
    finally:
        shutil.rmtree(state, ignore_errors=True)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (CannotRun, OSError, RuntimeError, KeyError, ValueError) as error:
        print(f"precision gate could not run: {type(error).__name__}: {error}")
        sys.exit(2)
