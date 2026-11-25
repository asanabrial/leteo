"""Measures each language's inflection set on the strict pass alone.

A query counts only when the answer came from the strict pass: no result marked
`partial`, none marked `semantic`, and no hint saying terms were corrected. The
prefix, substring, typo, widened, nearest and semantic stages all announce
themselves one of those ways, so a query one of them rescued scores as a miss
rather than as the stemmer's work. `canary` is what keeps that honest: if a
mangled word is ever answered without announcing itself, the set cannot say
what it claims to and the run stops.
"""
import json, os, shutil, sys, tempfile

# Standalone, a store of its own; under the ratchet, the ratchet's.
os.environ.setdefault("BENCH_STATE", tempfile.mkdtemp(prefix="leteo-inflection-"))

from inflection_sets import SETS
from mcpclient import MCP
from quality import PUBLISHED_LIMIT as LIMIT

PROJECT = "alpha-api"


class NotStrict(Exception):
    pass


def strict(reply):
    results = reply.get("results") or []
    if any(r.get("partial") or r.get("semantic") for r in results):
        return False
    return "corrected terms" not in (reply.get("hint") or "")


def search(server, query):
    text, _, raw = server.call("mem_search", {"query": query, "limit": LIMIT})
    if "error" in raw or (raw.get("result") or {}).get("isError"):
        raise RuntimeError(f"mem_search failed: {text[:300] or raw}")
    return json.loads(text)


def canary(server, spec):
    title = next(iter(spec["memories"].values()))[0]
    word = max(title.split(), key=len)
    mangled = word[:2] + word[1] + word[2:]
    reply = search(server, mangled)
    if reply.get("results") and strict(reply):
        raise NotStrict(f"{mangled!r} was answered and did not say it was relaxed; "
                        "the strict test cannot tell the stages apart")


def measure_language(code, spec, state, language=None):
    home = os.path.join(state, f"inflection-{code}")
    os.makedirs(home, exist_ok=True)
    with open(os.path.join(home, "settings.json"), "w") as settings:
        json.dump({"language": language or spec["language"]}, settings)
    server = MCP("leteo", PROJECT, home=home)
    try:
        ids = {}
        for key, (title, content) in spec["memories"].items():
            text, _, _ = server.call("mem_save", dict(title=title, content=content, type="decision"))
            reply = json.loads(text)
            if reply.get("status") != "inserted":
                raise RuntimeError(f"{code}/{key} was not inserted: {text[:300]}")
            ids[key] = reply["observation"]["id"]
        canary(server, spec)
        ranks = []
        for query, target in spec["queries"]:
            reply = search(server, query)
            found = [r["id"] for r in reply.get("results") or []]
            ranks.append(found.index(ids[target]) + 1 if strict(reply) and ids[target] in found else None)
            if "-v" in sys.argv:
                print(f"    {code} {query!r}: rank {ranks[-1]}, strict {strict(reply)}, found {len(found)}")
    finally:
        try:
            server.close()
        except Exception:
            server.p.kill()
    return dict(n=len(ranks), hit1=sum(r == 1 for r in ranks) / len(ranks),
                mrr=sum(1 / r for r in ranks if r) / len(ranks))


def measure(state, language=None):
    return {code: measure_language(code, spec, state, language) for code, spec in SETS.items()}


if __name__ == "__main__":
    if not os.environ.get("LETEO_BIN"):
        sys.exit("LETEO_BIN is not set; name the binary under test")
    state = os.environ["BENCH_STATE"]
    try:
        # `--porter-only` names English, which has no second stemmer: the same
        # binary and the same memories with only `porter` to answer them.
        for code, got in measure(state, "English" if "--porter-only" in sys.argv else None).items():
            print(f"{code}  n={got['n']}  hit@1 {got['hit1']:.3f}  MRR {got['mrr']:.3f}")
    finally:
        shutil.rmtree(state, ignore_errors=True)
