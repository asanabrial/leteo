import json, os, subprocess, tempfile, time

# Every store lives under BENCH_STATE and each engine is started with HOME and
# its data directory pointed there, so a run cannot reach the user's database.
BENCH = os.environ.get("BENCH_STATE") or os.path.join(tempfile.gettempdir(), "leteo-engram-bench")
ENGRAM = os.environ.get("ENGRAM_BIN", "engram")
LETEO = os.environ.get("LETEO_BIN", "leteo")
# The server is started from a directory under BENCH_STATE, so a relative path
# to the binary stops resolving the moment it is spawned. A bare name is left
# alone: that is a lookup on PATH, not a path.
if os.sep in LETEO:
    LETEO = os.path.abspath(LETEO)
os.makedirs(BENCH, exist_ok=True)

def engine_cmd(engine, project):
    cwd = os.path.join(BENCH, "work", project)
    os.makedirs(cwd, exist_ok=True)
    if engine == "engram":
        home = os.path.join(BENCH, "ehome")
        return [ENGRAM, "mcp"], cwd, dict(os.environ, HOME=home, ENGRAM_DATA_DIR=os.path.join(home, "data"))
    home = os.path.join(BENCH, "lhome")
    os.makedirs(home, exist_ok=True)
    return [LETEO, "mcp", "--database", os.path.join(home, "leteo.db")], cwd, dict(os.environ, HOME=home, LETEO_DATA_DIR=home)

class MCP:
    def __init__(self, engine, project):
        cmd, cwd, env = engine_cmd(engine, project)
        self.p = subprocess.Popen(cmd, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, bufsize=1)
        self.n = 0
        self.req("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                                "clientInfo": {"name": "bench", "version": "0"}})
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
    def req(self, method, params):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        while True:
            line = self.p.stdout.readline()
            if not line: raise RuntimeError("server closed")
            msg = json.loads(line)
            if msg.get("id") == self.n: return msg
    def call(self, tool, args):
        t0 = time.perf_counter()
        r = self.req("tools/call", {"name": tool, "arguments": args})
        dt = time.perf_counter() - t0
        res = r.get("result") or {}
        text = "".join(c.get("text", "") for c in res.get("content", []))
        return text, dt, r
    def close(self):
        self.p.stdin.close(); self.p.wait(timeout=5)
