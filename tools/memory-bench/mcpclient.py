import json, os, subprocess, tempfile, time

# Every store lives under BENCH_STATE and each engine is started with HOME and
# its data directory pointed there, so a run cannot reach the user's database.
BENCH = os.environ.get("BENCH_STATE") or os.path.join(tempfile.gettempdir(), "leteo-memory-bench")
ENGRAM = os.environ.get("ENGRAM_BIN", "engram")
LETEO = os.environ.get("LETEO_BIN", "leteo")
# The server is started from a directory under BENCH_STATE, so a relative path
# to the binary stops resolving the moment it is spawned. A bare name is left
# alone: that is a lookup on PATH, not a path. Windows accepts `/` as well as the
# native separator, so `target/release/leteo.exe` is a path there too.
if os.sep in LETEO or (os.altsep and os.altsep in LETEO):
    LETEO = os.path.abspath(LETEO)
os.makedirs(BENCH, exist_ok=True)

# The bench starts Engram dozens of times, and its startup update check is a
# network call that changes nothing measured here. One mapping so every place
# that builds an Engram environment turns it off the same way rather than
# carrying its own copy of the variable name and value.
ENGRAM_QUIET_ENV = {"ENGRAM_NO_UPDATE_CHECK": "1"}

def engine_cmd(engine, project, home=None, binary=None):
    # A relative path passed by the gate has the same problem the module-level
    # path had: the server is spawned from `cwd` below, where it will not
    # resolve. A bare name stays a PATH lookup.
    if binary is not None and (os.sep in binary or (os.altsep and os.altsep in binary)):
        binary = os.path.abspath(binary)
    cwd = os.path.join(BENCH, "work", project)
    os.makedirs(cwd, exist_ok=True)
    if engine == "engram":
        home = os.path.join(BENCH, "ehome")
        return [binary or ENGRAM, "mcp"], cwd, dict(os.environ, HOME=home, ENGRAM_DATA_DIR=os.path.join(home, "data"), **ENGRAM_QUIET_ENV)
    home = home or os.path.join(BENCH, "lhome")
    os.makedirs(home, exist_ok=True)
    env = dict(os.environ, HOME=home, LETEO_DATA_DIR=home)
    # The server runs from `cwd` above, so a relative model directory would be
    # looked for under the bench state and not where the person who typed it
    # meant. The stage then stays off without a word, which is how a documented
    # `LETEO_MODEL_DIR=assets/model` made a run measure the lexical search alone.
    if env.get("LETEO_MODEL_DIR"):
        env["LETEO_MODEL_DIR"] = os.path.abspath(env["LETEO_MODEL_DIR"])
    # `binary` is how the two-binary gate starts a specific build; the module
    # name is the one a caller that names none has always used, and the path
    # resolution above happens at import for it only.
    return [binary or LETEO, "mcp", "--database", os.path.join(home, "leteo.db")], cwd, env

class MCP:
    def __init__(self, engine, project, home=None, launch=None):
        # `launch` is a ready (cmd, cwd, env) for an engine `engine_cmd` does not
        # know; the adapters build theirs so every isolation variable lives in
        # one file rather than being split between here and there.
        cmd, cwd, env = launch or engine_cmd(engine, project, home)
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
        self.p.stdin.close()
        # `uvx` and `npx` start the server as a child, and a wrapper that has
        # not exited within the grace period would otherwise leave it running
        # against the throwaway store for the rest of the machine's uptime.
        try:
            self.p.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.p.kill(); self.p.wait()


class UnusableReply(RuntimeError):
    """A reply that is not an answer.

    A JSON-RPC error, a result carrying `isError`, or text that will not parse is
    not an empty answer. A measurement that folded one into the empty count would
    improve its no-answer figures on an engine fault — the exact direction those
    figures exist to check.
    """


def call_json(m, tool, args):
    """One tool call, or `UnusableReply`.

    The single place a reply is judged usable, so `ratchet.py` and
    `bench_search.py` cannot come to different conclusions about what an answer
    is. A `RuntimeError` subclass, so a caller that already catches that keeps
    working.
    """
    text, dt, raw = m.call(tool, args)
    if "error" in raw or (raw.get("result") or {}).get("isError"):
        raise UnusableReply(f"{tool} failed: {text[:300] or raw}")
    try:
        return json.loads(text), text, dt
    except ValueError:
        raise UnusableReply(f"{tool} answered something that is not JSON: {text[:300]}")
