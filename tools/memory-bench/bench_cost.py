import json, os, shutil, statistics as st, subprocess, time, uuid
from mcpclient import MCP, BENCH, ENGRAM, LETEO, ENGRAM_QUIET_ENV
from quality import PUBLISHED_LIMIT
PORT = "17437"
bindir = os.path.join(BENCH, "bin"); os.makedirs(bindir, exist_ok=True)
if not os.path.exists(os.path.join(bindir, "engram")): os.symlink(ENGRAM, os.path.join(bindir, "engram"))
EHOME, LHOME = os.path.join(BENCH, "ehome"), os.path.join(BENCH, "lhome")
eenv = dict(os.environ, HOME=EHOME, ENGRAM_DATA_DIR=os.path.join(EHOME, "data"), ENGRAM_PORT=PORT,
            PATH=bindir + ":" + os.environ["PATH"], **ENGRAM_QUIET_ENV)
lenv = dict(os.environ, HOME=LHOME, LETEO_DATA_DIR=LHOME, LETEO_DATABASE=os.path.join(LHOME, "leteo.db"))
cwd = os.path.join(BENCH, "work", "alpha-api")
def pct(xs): xs = sorted(xs); return st.median(xs), xs[int(0.9 * len(xs))]

print("== reply bytes (project alpha-api, 85 memories) ==")
for e in ("engram", "leteo"):
    m = MCP(e, "alpha-api")
    for q in ("service", "latency"):
        t, _, _ = m.call("mem_search", {"query": q, "limit": PUBLISHED_LIMIT})
        d = json.loads(t); print(f"{e} mem_search '{q}' limit={PUBLISHED_LIMIT}: {len(d.get('results') or [])} results, {len(t.encode())} bytes")
    t, _, _ = m.call("mem_context", {}); print(f"{e} mem_context (defaults): {len(t.encode())} bytes")
    m.close()

srv = subprocess.Popen([ENGRAM, "serve", PORT], env=eenv, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
time.sleep(1.5)
# Engram's hook is a script in its source tree, not in the binary, and runs as
# shipped against a running `engram serve` on a private port.
hook = os.path.join(os.environ["ENGRAM_SRC"], "plugin", "claude-code", "scripts", "session-start.sh")
def run_hook(e):
    payload = json.dumps({"session_id": str(uuid.uuid4()), "cwd": cwd, "hook_event_name": "SessionStart", "source": "startup"})
    cmd, env = ([hook], eenv) if e == "engram" else ([LETEO, "hook", "session-start"], lenv)
    t0 = time.perf_counter(); r = subprocess.run(cmd, input=payload, env=env, cwd=cwd, capture_output=True, text=True)
    return (time.perf_counter() - t0) * 1000, r.stdout, r.returncode
try:
    print("== session-start hook ==")
    for e in ("engram", "leteo"):
        ms, out, rc = run_hook(e)
        open(os.path.join(BENCH, f"hook_{e}.txt"), "w").write(out)
        lat = [run_hook(e)[0] for _ in range(30)]
        print(f"{e}: rc={rc} first-run stdout {len(out.encode())} bytes; latency median {pct(lat)[0]:.1f} ms p90 {pct(lat)[1]:.1f} ms (n=30)")
    print("== cold CLI search (process start + query), n=30 ==")
    for e in ("engram", "leteo"):
        cmd = [ENGRAM, "search", "connection pool", "--project", "alpha-api"] if e == "engram" else \
              [LETEO, "search", "connection pool", "--project", "alpha-api"]
        lat = []
        for _ in range(30):
            t0 = time.perf_counter(); subprocess.run(cmd, env=eenv if e == "engram" else lenv, cwd=cwd, capture_output=True)
            lat.append((time.perf_counter() - t0) * 1000)
        print(f"{e}: median {pct(lat)[0]:.1f} ms p90 {pct(lat)[1]:.1f} ms")
finally:
    srv.terminate(); srv.wait()

# What the two hooks beside the session-start one put in the model's context.
# They are measured by `session_cost.py` over throwaway copies of the stores
# above, so the stores this script was pointed at are not written to a second
# time. In tokens where tiktoken is installed, and in bytes/4 when it is not,
# with the unit printed either way.
import pathlib
import tempfile

import session_cost as sc

count, unit = sc.make_counter()
prompts = sc.read_lines(sc.DATA / "prompts.txt")
print(f"== per-prompt hook ({len(prompts)} prompts) and post-compaction hook, in {unit} ==")
with tempfile.TemporaryDirectory(prefix="leteo-bench-cost-") as w:
    work = pathlib.Path(w)
    plugin = os.path.join(os.environ["ENGRAM_SRC"], "plugin", "claude-code")
    for e in ("engram", "leteo"):
        store = os.path.join(EHOME, "data", "engram.db") if e == "engram" else os.path.join(LHOME, "leteo.db")
        eng = sc.Engine(e, ENGRAM if e == "engram" else LETEO, store, work, "alpha-api",
                        plugin_root=plugin if e == "engram" else None)
        eng.start()
        try:
            h = sc.hooks_measure(eng, prompts, count, {})
        finally:
            eng.stop()
        spoke = sum(1 for x in h["prompts"] if x)
        print(f"{e}: per-prompt hook {sum(h['prompts'])} over the session, spoke on {spoke}/{len(prompts)}"
              f" (max {max(h['prompts'])}); post-compaction {h['post_compaction']}"
              f"; per-prompt latency median {h['prompt_ms_median']} ms")
