# match_mode=any is opt-in, so this is not the answer an agent gets by default.
import json, os
from comparison_corpus import queries
from mcpclient import MCP, BENCH
from quality import PUBLISHED_LIMIT
keymap = json.load(open(os.path.join(BENCH, "keymap_engram.json")))
from collections import defaultdict
agg = defaultdict(list)
for project in ("alpha-api", "beta-web"):
    m = MCP("engram", project)
    for q in [q for q in queries() if q["project"] == project]:
        t, _, _ = m.call("mem_search", {"query": q["q"], "limit": PUBLISHED_LIMIT, "match_mode": "any"})
        ids = [r["id"] for r in json.loads(t).get("results") or []]
        tgt = keymap[q["target"]]; r = ids.index(tgt) + 1 if tgt in ids else None
        agg[q["kind"]].append(r); agg["ALL"].append(r)
    m.close()
for k, rs in sorted(agg.items()):
    n = len(rs); print(f"{k:11} h@1 {sum(r==1 for r in rs)/n:.2f} h@5 {sum(bool(r) and r<=5 for r in rs)/n:.2f} MRR {sum(1/r for r in rs if r)/n:.3f}")
