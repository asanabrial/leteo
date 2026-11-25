import json, os, statistics as st
from collections import defaultdict
from comparison_corpus import no_answer, queries
from mcpclient import MCP, BENCH, call_json
from quality import PUBLISHED_LIMIT
keymap = json.load(open(os.path.join(BENCH, "keymap_engram.json")))  # Leteo adoption keeps ids (checked: same id+sync_id)


def measure():
    out = {}
    for engine in ("engram", "leteo"):
        rows = []
        for project in ("alpha-api", "beta-web"):
            m = MCP(engine, project)
            m.call("mem_search", {"query": "warmup"})
            for q in [q for q in queries() if q["project"] == project]:
                # Through `call_json`, so a reply carrying an error or one that
                # will not parse fails the run instead of being recorded as `n=0`
                # and read as a question the engine could not answer. An engine
                # fault is not a no-answer, and counting it as one improves the
                # no-answer figures this script exists to compare.
                reply, text, dt = call_json(m, "mem_search", {"query": q["q"], "limit": PUBLISHED_LIMIT})
                ids = [r["id"] for r in reply.get("results") or []]
                tgt = keymap[q["target"]]
                rank = ids.index(tgt) + 1 if tgt in ids else None
                rows.append(dict(q, rank=rank, n=len(ids), bytes=len(text.encode()), ms=dt * 1000))
            m.close()
        out[engine] = rows
        # The questions the corpus cannot answer, for both engines: how many come
        # back with something, how many of those carry no caveat, and how big the
        # replies are. A stage with nothing to give should say less, not more.
        na = []
        for project in ("alpha-api", "beta-web"):
            m = MCP(engine, project)
            m.call("mem_search", {"query": "warmup"})
            for q in [q for q in no_answer() if q["project"] == project]:
                reply, text, dt = call_json(m, "mem_search", {"query": q["q"], "limit": PUBLISHED_LIMIT})
                na.append(dict(q, n=len(reply.get("results") or []), bytes=len(text.encode()),
                               caveat=bool(reply.get("hint")), ms=dt * 1000))
            m.close()
        out["no_answer_" + engine] = na
    return out


def report(out):
    def summ(rows):
        n = len(rows)
        return (sum(r["rank"] == 1 for r in rows) / n, sum(bool(r["rank"]) and r["rank"] <= 5 for r in rows) / n,
                sum(1 / r["rank"] for r in rows if r["rank"]) / n, n)
    kinds = sorted({r["kind"] for r in out["engram"]})
    print(f"{'kind':12} {'n':>3} | {'E h@1':>6} {'E h@5':>6} {'E MRR':>6} | {'L h@1':>6} {'L h@5':>6} {'L MRR':>6}")
    for k in kinds + ["ALL"]:
        e = summ([r for r in out["engram"] if k == "ALL" or r["kind"] == k])
        l = summ([r for r in out["leteo"] if k == "ALL" or r["kind"] == k])
        print(f"{k:12} {e[3]:>3} | {e[0]:6.2f} {e[1]:6.2f} {e[2]:6.3f} | {l[0]:6.2f} {l[1]:6.2f} {l[2]:6.3f}")
    for e in ("engram", "leteo"):
        ms = sorted(r["ms"] for r in out[e]); b = [r["bytes"] for r in out[e]]; n = [r["n"] for r in out[e]]
        full = [r["bytes"] for r in out[e] if r["n"] == 20]
        print(f"{e}: warm MCP search median {st.median(ms):.1f} ms p90 {ms[int(.9*len(ms))]:.1f} ms; "
              f"reply bytes mean {st.mean(b):.0f}; mean results {st.mean(n):.1f}; zero-result queries {sum(x==0 for x in n)}; "
              f"replies with 20 results: {len(full)} mean bytes {st.mean(full) if full else 0:.0f}")
        na = out["no_answer_" + e]
        answered = [r for r in na if r["n"]]
        print(f"{e}: no-answer {len(answered)}/{len(na)} answered, "
              f"{sum(not r['caveat'] for r in answered)} with no caveat, "
              f"mean bytes {st.mean(r['bytes'] for r in answered) if answered else 0:.0f}")


def main():
    out = measure()
    json.dump(out, open(os.path.join(BENCH, "results_search.json"), "w"), ensure_ascii=False, indent=1)
    report(out)
    return 0


if __name__ == "__main__":
    # `UnusableReply` is a `RuntimeError`; named here so the failure this script
    # was hardened against is the one a reader sees first.
    try:
        code = main()
    except (OSError, RuntimeError, KeyError, ValueError) as e:
        print(f"bench_search could not run: {type(e).__name__}: {e}")
        code = 2
    raise SystemExit(code)
