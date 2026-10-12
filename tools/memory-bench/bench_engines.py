#!/usr/bin/env python3
"""Ask every engine of a tier the same held-out queries, through its own adapter.

Writes `results_engines.json` under BENCH_STATE for `compare.py`. An engine that
cannot be measured is recorded with the reason and no rows: the page lists it as
not measured, never as a row of zeros.

    BENCH_STATE=/tmp/state LETEO_BIN=target/release/leteo python3 bench_engines.py [--tier deterministic|llm] [--only engram,leteo] [--passes 3]
"""

import argparse
import datetime
import json
import os
import shutil
import sys
import time

from adapters import DETERMINISTIC, Skipped
from comparison_corpus import corpus, no_answer, queries
from mcpclient import BENCH

# The limit every engine is asked at. An engine with no limit parameter returns
# what it returns and the harness keeps the first LIMIT, so no engine is scored
# on more than the others were.
LIMIT = 20


def measure(adapter, members):
    rows, na = [], []
    for q in queries():
        hits = adapter.search(q["project"], q["q"], LIMIT)[:LIMIT]
        mapped = [adapter.map_hit(h, [m for m in members if m["project"] == q["project"]]) for h in hits]
        rank = mapped.index(q["target"]) + 1 if q["target"] in mapped else None
        rows.append(dict(q, rank=rank, n=len(hits), unmappable=sum(k is None for k in mapped)))
    for q in no_answer():
        hits = adapter.search(q["project"], q["q"], LIMIT)[:LIMIT]
        na.append(dict(q, n=len(hits)))
    return rows, na


def engine_entry(cls, date, required, **extra):
    """The card an engine's result is recorded on, before it is measured."""
    return dict(name=cls.name, label=cls.label, date=date, required=required, status="measured",
                reason="", version="", rows=[], no_answer=[], **extra)


def ending(e):
    """The status and the fault an exception leaves an engine's run in.

    A `Skipped` is a prerequisite that is absent and anything else is a run that
    started and then faulted; `skipped` and `failed` must not be read as the same
    thing. One rule, so the deterministic tier and the LLM tier cannot drift on
    what the same exception means.
    """
    if isinstance(e, Skipped):
        return "skipped", str(e)
    return "failed", f"{type(e).__name__}: {e}"


def teardown(adapter, fault):
    """Close an engine's store and report what closing it cost.

    A `close` that raises would run in the runner's `finally`, replace the fault
    the `except` had already caught, and leave the runner before the entry was
    written -- so the engine would carry no status, no reason, and no page. The
    fault is returned instead, along with whether closing is what produced it: the
    LLM tier keeps a partial run when a pass faults, and a store that was never
    shut down is not that.
    """
    try:
        adapter.close()
    except Exception as e:  # noqa: BLE001 - a teardown fault is named, never scored
        return fault or e, fault is None
    return fault, False


def run_one(cls, state, members, date):
    adapter = cls(state)
    entry = engine_entry(cls, date, adapter.required)
    fault = None
    try:
        adapter.check()
        entry["version"] = adapter.version()
        adapter.load(members)
        entry["rows"], entry["no_answer"] = measure(adapter, members)
    except Exception as e:  # noqa: BLE001 - any engine fault is named, never scored
        fault = e
    finally:
        fault, _ = teardown(adapter, fault)
    if fault is not None:
        # A crash is not a missing prerequisite: `skipped` says the engine could
        # not start, and a run that started and then faulted has to be readable
        # as that on the page rather than as a skip with a long reason.
        status, detail = ending(fault)
        entry.update(status=status, reason=detail if status == "skipped" else f"could not run: {detail}",
                     rows=[], no_answer=[])
    return entry


def run_repeated(cls, state, members, date, passes):
    """One deterministic-tier engine whose store is not the same twice.

    Each pass starts from an empty store — a state directory of its own, or the
    one the adapter's `load` wipes and rewrites — so what one pass left behind
    cannot answer for the next. The first pass's rows are kept as `rows`, so the
    tables that read a single measurement still have one.

    A run that does not take every pass it planned is not a measurement at all:
    keeping the passes that survived would rank an engine on a truncated one,
    which is the single draw this exists to stop presenting as reproducible. It
    is reported as the fault it is instead.
    """
    entry = engine_entry(cls, date, cls.required, passes=[])
    fault = None
    for k in range(1, passes + 1):
        adapter = cls(os.path.join(state, f"pass-{k}"))
        try:
            adapter.check()
            if not entry["version"]:
                entry["version"] = adapter.version()
            adapter.load(members)
            rows, na = measure(adapter, members)
            entry["passes"].append(dict(rows=rows, no_answer=na))
            if k == 1:
                entry["rows"], entry["no_answer"] = rows, na
        except Exception as e:  # noqa: BLE001 - any engine fault is named, never scored
            fault = e
        finally:
            fault, _ = teardown(adapter, fault)
        if fault is not None:
            break
    if fault is not None or len(entry["passes"]) != passes:
        # A `Skipped` after a pass has completed is not an absent prerequisite:
        # the engine started, answered that pass and then stopped, and the page
        # has to read that as a fault rather than as a machine that never had the
        # engine at all. Either way the run is not a measurement, so it carries no
        # row, and the reason says how far it got.
        status, detail = ending(fault)
        if entry["passes"]:
            status = "failed"
            reason = f"stopped after {len(entry['passes'])} of {passes} passes: {detail}"
        else:
            reason = detail if status == "skipped" else f"could not run: {detail}"
        entry.update(status=status, reason=reason, rows=[], no_answer=[], passes=[])
    return entry


# Passes of the LLM tier. A model that extracts facts or chooses entities does
# not give the same store twice, so one pass is one draw and the page reports the
# spread of several.
DEFAULT_PASSES = 3

# A run that fills the disk takes the machine's other work down with it, which
# happened once. Below this much free space no further pass is started.
MIN_FREE_BYTES = 8 * 1024 ** 3


def run_llm(cls, state, members, date, passes):
    """One LLM-tier engine: `passes` independent loads, each from an empty store.

    Every pass has a state directory of its own, so what one pass left behind
    cannot answer for the next. The first fault ends the engine's run: the passes
    that completed are kept and the fault is named, because a second attempt at
    something that just failed would spend the same hours to learn the same thing.
    """
    entry = engine_entry(cls, date, False, passes=[], failed="", models={}, planned=passes)
    failure = None
    fault = None
    for k in range(1, passes + 1):
        adapter = cls(os.path.join(state, f"pass-{k}"))
        try:
            free = shutil.disk_usage(state).free
            if free < MIN_FREE_BYTES:
                raise RuntimeError(f"stopped before pass {k}: {free / 1024 ** 3:.1f} GiB of disk free, "
                                   f"under the {MIN_FREE_BYTES // 1024 ** 3} GiB this run needs to leave")
            adapter.check()
            if not entry["version"]:
                entry["version"] = adapter.version()
                entry["models"] = adapter.models()
                entry["ollama"] = adapter.ollama.get("version", "")
            started = time.time()
            adapter.load(members)
            loaded = time.time() - started
            rows, na = measure(adapter, members)
            entry["passes"].append(dict(rows=rows, no_answer=na, load_seconds=round(loaded), notes=adapter.notes))
        except Exception as e:  # noqa: BLE001 - any engine fault is named, never scored
            status, detail = ending(e)
            failure = status, detail
            entry["failed"] = f"pass {k}: {detail}"
            fault = e
        finally:
            fault, torn = teardown(adapter, fault)
        if fault is not None:
            if not entry["failed"]:
                status, detail = ending(fault)
                failure = status, detail
                entry["failed"] = f"pass {k}: {detail}"
            if torn:
                # A store that was never shut down is not a completed run, so the
                # passes it took are not kept, whatever this tier does with a pass
                # that merely faulted.
                entry["passes"] = []
            break
    if not entry["passes"]:
        status, detail = failure or ("skipped", "")
        entry.update(status=status,
                     reason=detail if status == "skipped" else f"could not run: {entry['failed']}")
    return entry


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument("--tier", default="deterministic", choices=("deterministic", "llm"))
    ap.add_argument("--only", help="comma-separated engine names, for working on one adapter")
    ap.add_argument("--passes", type=int, default=DEFAULT_PASSES, help="LLM tier only: independent loads per engine")
    args = ap.parse_args(argv)
    if args.passes < 1:
        ap.error("--passes must be at least 1")
    if args.tier == "llm":
        # Imported here: it installs signal handlers that remove containers, which
        # a run that never starts one has no business doing.
        from llm_tier import LLM as adapters
    else:
        adapters = DETERMINISTIC
    only = set(args.only.split(",")) if args.only else None
    members = corpus()
    date = datetime.date.today().isoformat()
    out = []
    for cls in adapters:
        if only and cls.name not in only:
            continue
        if args.tier == "llm":
            entry = run_llm(cls, BENCH, members, date, args.passes)
        elif getattr(cls, "passes", 1) > 1:
            # The default is for a stand-in adapter that is not an `Adapter`
            # subclass; every shipped one inherits `passes` from the base class.
            entry = run_repeated(cls, BENCH, members, date, cls.passes)
        else:
            entry = run_one(cls, BENCH, members, date)
        note = entry["reason"] or (f"{len(entry['passes'])} of {entry['planned']} passes; {entry['failed']}" if entry.get("failed") else "")
        print(f"{entry['label']}: {entry['status']}{' - ' + note if note else ''}", file=sys.stderr)
        out.append(entry)
    path = os.path.join(BENCH, "results_engines.json" if args.tier == "deterministic" else "results_llm.json")
    if args.tier == "llm" and only and os.path.exists(path):
        # An engine that takes hours is run on its own. The others keep the rows
        # and the run date they were measured with; the page prints both, so a
        # row from an earlier run says so.
        kept = [e for e in json.load(open(path))["engines"] if e["name"] not in only]
        out = kept + out
    json.dump(dict(tier=args.tier, engines=out), open(path, "w"), ensure_ascii=False, indent=1)
    missing = [e["label"] for e in out if e["required"] and e["status"] != "measured"]
    if missing:
        print(f"required engine not measured: {', '.join(missing)}", file=sys.stderr)
        return 2
    if not any(e["status"] == "measured" for e in out):
        # A run that measured nothing has no numbers to publish, and a comparison
        # that "succeeded" having executed nothing guards nothing. Every LLM-tier
        # engine is non-required, so this is also what refuses a tier whose
        # engines all skipped rather than returning a silent success.
        print("no engine was measured", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
