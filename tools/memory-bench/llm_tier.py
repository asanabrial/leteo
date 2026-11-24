"""The opt-in tier: engines that need a local language model.

Everything here runs against a local Ollama and nothing else. No paid API is
called, and the corpus is only ever sent to a loopback address, so a developer
who exports `BENCH_OLLAMA_URL` at a remote host is refused rather than obeyed.

An engine that cannot run here -- Ollama down, a model not pulled, Docker or a
package manager absent -- raises `Skipped` with the reason, which the page prints
as "not measured". It is never a row of zeros: a zero is a measurement.
"""

import atexit
import json
import os
import queue
import re
import shutil
import signal
import socket
import subprocess
import threading
import time
import urllib.parse
import urllib.request

from adapters import Adapter, Hit, Skipped, isolated_env, package_cache_env, stored_titles
from mcpclient import BENCH, MCP, call_json

HERE = os.path.dirname(os.path.abspath(__file__))
# Installed tools are kept across passes and runs; what a pass writes is not.
TOOLS = os.path.join(BENCH, "llm-tools")

OLLAMA_URL = os.environ.get("BENCH_OLLAMA_URL", "http://localhost:11434").rstrip("/")
LLM_MODEL = os.environ.get("BENCH_LLM_MODEL", "qwen2.5:7b")
EMBED_MODEL = os.environ.get("BENCH_EMBED_MODEL", "nomic-embed-text")

# The only hosts a corpus may be sent to. `host.docker.internal` is how a
# container reaches the Ollama on the same machine.
LOCAL_HOSTS = ("localhost", "127.0.0.1", "::1", "host.docker.internal")

# Every wait in this module is a bound on something local, and each is named with
# the reason behind its value: a bare number cannot tell a later change whether it
# was generous or tight.

# Ollama answers its own version and tag listing on the loopback at once; the only
# slow call is the first while the daemon is warming, which this is well clear of.
OLLAMA_SECONDS = 15

# The first call to an embedder loads its weights into the daemon, so the width
# probe is given room for a cold load rather than a warm round trip.
EMBEDDING_SECONDS = 120

# `uv run --no-project` resolves and installs the pinned SDK on first use. The
# environment is shared across passes and runs, so only the first ever pays it.
UV_RUN_SECONDS = 600

# claude-mem fills its vector index in the background after `save_memory` returns,
# one pass of the corpus at a time.
CLAUDE_MEM_INDEX_SECONDS = 600

# claude-mem's worker answers only once Chroma has downloaded and loaded its
# embedder; the first calls after launch fail for that reason and are retried.
CLAUDE_MEM_RETRY_SECONDS = 10

# `npm install --prefix` fetches the pinned claude-mem package and its tree; the
# package cache is shared across passes and runs, so only a cold first install
# pays it.
CLAUDE_MEM_INSTALL_SECONDS = 900

# One mem0 `add` with inference runs the model and fills the store; a pass of 91
# memories took about 11 minutes on the machine that produced the page, so this is
# generous by design and still names a hang before the hour is spent.
MEM0_ADD_SECONDS = 900

# One mem0 `search` embeds the query and reads the store with no extraction, so it
# is bounded apart from the write budget.
MEM0_SEARCH_SECONDS = 300

# FalkorDB answers PING as soon as it is up; the wait is the container's own start,
# not the graph engine behind it.
FALKORDB_READY_SECONDS = 120


def require_local(url):
    host = urllib.parse.urlparse(url).hostname or ""
    if host not in LOCAL_HOSTS:
        raise Skipped(f"BENCH_OLLAMA_URL names `{host}`, which is not this machine; the LLM tier is local-only "
                      "and would otherwise send the corpus off it")


def ollama_get(path, url=None):
    with urllib.request.urlopen((url or OLLAMA_URL) + path, timeout=OLLAMA_SECONDS) as r:
        return json.load(r)


def resolve_models(tags, wanted):
    """The installed record of each wanted model, or `Skipped` naming the absent ones.

    `qwen2.5:7b` must be that tag; a bare name such as `nomic-embed-text` is the
    `latest` tag, which is how Ollama itself reads it. The digest is kept because
    a tag can be re-pointed, and a result belongs to the weights that produced it.
    """
    by_name = {t["name"]: t for t in tags}
    found, missing = {}, []
    for want in wanted:
        tag = by_name.get(want if ":" in want else want + ":latest")
        if tag is None:
            missing.append(want)
            continue
        d = tag.get("details") or {}
        found[want] = dict(name=tag["name"], digest=tag["digest"][:12],
                           size=d.get("parameter_size", ""), quantization=d.get("quantization_level", ""))
    if missing:
        raise Skipped(f"Ollama does not have {', '.join(missing)}; `ollama pull {' '.join(missing)}` and run again")
    return found


def ollama_state(wanted, url=None):
    """`(ollama version, {model: record})`, or `Skipped`."""
    url = (url or OLLAMA_URL).rstrip("/")
    require_local(url)
    try:
        version = ollama_get("/api/version", url)["version"]
        tags = ollama_get("/api/tags", url)["models"]
    except (OSError, ValueError, KeyError) as e:
        raise Skipped(f"Ollama is not reachable at {url} ({type(e).__name__}); start it with `ollama serve`")
    return version, resolve_models(tags, wanted)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def need_tool(name, why):
    if not shutil.which(name):
        raise Skipped(f"`{name}` was not found on PATH; it is needed {why}")


def uv_env():
    """uv's cache and its managed interpreters, resolved in the caller's real
    environment so that the redirected HOME does not download them again."""
    env = dict(package_cache_env())
    py = subprocess.run(["uv", "python", "dir"], capture_output=True, text=True)
    if py.returncode == 0 and py.stdout.strip():
        env["UV_PYTHON_INSTALL_DIR"] = py.stdout.strip()
    return env


# ---------------------------------------------------------------------------
# Throwaway containers.
#
# The machine this runs on holds other people's containers and databases, and the
# only thing this module may remove is what it started. Every container carries a
# name made of this prefix and the pid, binds a random loopback port, mounts no
# named volume, and is removed with its anonymous volumes (`rm -f -v`) when the
# adapter closes, when the process exits and when it is interrupted.
# ---------------------------------------------------------------------------

CONTAINER_PREFIX = "leteo-bench-"
_STARTED = []


def container_name(suffix):
    return f"{CONTAINER_PREFIX}{os.getpid()}-{suffix}"


def run_command(image, suffix, ports, env=None, args=(), interactive=False):
    cmd = ["docker", "run", "-i" if interactive else "-d", "--name", container_name(suffix), "--label", "leteo-bench=1"]
    for p in ports:
        cmd += ["-p", f"127.0.0.1::{p}"]
    for k, v in (env or {}).items():
        cmd += ["-e", f"{k}={v}"]
    return cmd + [image, *args]


def remove_command(name):
    return ["docker", "rm", "-f", "-v", name]


def docker_ready():
    need_tool("docker", "to start the throwaway database container")
    r = subprocess.run(["docker", "info", "--format", "{{.ServerVersion}}"], capture_output=True, text=True, timeout=60)
    if r.returncode:
        raise Skipped("the Docker daemon is not reachable; start Docker to measure this engine")


def _record(name):
    """Arm cleanup and list a container as ours, before the daemon is asked for it.

    Done before the start: an interruption between the daemon creating the
    container and this process hearing about it would otherwise leave it behind.
    """
    _arm()
    _STARTED.append(name)
    return name


def start_container(image, suffix, ports, env=None, args=()):
    """Start one container; return `{container port: host port}`."""
    name = _record(container_name(suffix))
    r = subprocess.run(run_command(image, suffix, ports, env, args), capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError(f"`docker run {image}` failed: {(r.stderr or r.stdout).strip()[-300:]}")
    mapped = {}
    for p in ports:
        out = subprocess.run(["docker", "port", name, f"{p}/tcp"], capture_output=True, text=True).stdout
        m = re.search(r":(\d+)\s*$", out.strip().splitlines()[0]) if out.strip() else None
        if not m:
            raise RuntimeError(f"no host port was published for {p}/tcp of {name}")
        mapped[p] = int(m.group(1))
    return name, mapped


def track_container(suffix):
    """Arm cleanup for a container this process is about to start itself."""
    return _record(container_name(suffix))


def remove_container(name):
    subprocess.run(remove_command(name), capture_output=True, text=True)
    if name in _STARTED:
        _STARTED.remove(name)


def _remove_all():
    for name in list(_STARTED):
        remove_container(name)


_ARMED = []


def _arm():
    """Remove the containers on exit and on interruption, from the first start on.

    Done when a container is first started and not at import, so that importing
    this module for its pure functions does not take over the process's signals.
    """
    if _ARMED:
        return
    _ARMED.append(True)
    atexit.register(_remove_all)
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda n, f: (_remove_all(), os._exit(128 + n)))


def wait_until(probe, seconds, what):
    end = time.time() + seconds
    last = None
    while time.time() < end:
        try:
            if probe():
                return
        except Exception as e:  # noqa: BLE001 - the service is still starting
            last = e
        time.sleep(1)
    raise RuntimeError(f"{what} was not ready after {seconds}s" + (f" ({last})" if last else ""))


# ---------------------------------------------------------------------------
# A worker process spoken to in JSON lines, for engines that are libraries.
# ---------------------------------------------------------------------------

class LineWorker:
    def __init__(self, cmd, cwd, env, log):
        os.makedirs(os.path.dirname(log), exist_ok=True)
        self._log = open(log, "w")
        self.p = subprocess.Popen(cmd, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=self._log, text=True, bufsize=1)
        self._lines = queue.Queue()
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self):
        for line in self.p.stdout:
            self._lines.put(line)
        self._lines.put(None)

    def send(self, obj, timeout):
        self.p.stdin.write(json.dumps(obj, ensure_ascii=False) + "\n")
        self.p.stdin.flush()
        try:
            line = self._lines.get(timeout=timeout)
        except queue.Empty:
            raise RuntimeError(f"the worker did not answer within {timeout}s")
        if line is None:
            raise RuntimeError("the worker exited; see its log under BENCH_STATE")
        reply = json.loads(line)
        if "error" in reply:
            raise RuntimeError(reply["error"])
        return reply

    def close(self):
        try:
            self.p.stdin.close()
            self.p.wait(timeout=10)
        except (OSError, subprocess.TimeoutExpired):
            self.p.kill()
            self.p.wait()
        self._log.close()


# ---------------------------------------------------------------------------
# Adapters
# ---------------------------------------------------------------------------

# Engines a default run lists as not measured, each with the variable that opts in
# and what was measured on the machine that produced the page. One table, read by
# the adapters and by their tests, so the rule and its reason live in one place.
OPT_IN = {
    "mem0-infer": ("BENCH_MEM0_INFER",
                   "a pass on qwen2.5:7b takes about 11 minutes, and on the 24 GB machine that produced this page it "
                   "pushed swap to about 20 GB and filled the disk before three passes could finish"),
    "graphiti": ("BENCH_GRAPHITI",
                 "on qwen2.5:7b in json_object mode Graphiti's own validation rejected 16 of the first 18 episodes "
                 "(EdgeDuplicate and Organization errors) at about 45 s per episode, so one pass of 91 memories "
                 "exceeds an hour"),
}


def gate(name):
    """`Skipped` with the measured reason unless the engine's variable is exactly `1`."""
    var, why = OPT_IN.get(name, (None, None))
    if var and os.environ.get(var) != "1":
        raise Skipped(f"not run by default: {why}; set {var}=1 to run it anyway")


class LLMAdapter(Adapter):
    """An engine whose write or search path calls a local model.

    `models_used` names what the row really used, because a row that calls no
    model must not be reported as if it had: it is claimed in the page's model
    column, and a false entry there would put a digest beside a result it did not
    produce.
    """

    models_used = ()
    notes_text = ()

    def __init__(self, state):
        super().__init__(state)
        self.ollama = {}
        self.notes = list(self.notes_text)

    def wanted_models(self):
        return {"llm": LLM_MODEL, "embed": EMBED_MODEL}

    def check(self):
        wanted = self.wanted_models()
        used = [wanted[k] for k in self.models_used]
        if used:
            version, models = ollama_state(used)
            self.ollama = dict(version=version, models=models)
        self.check_tools()

    def check_tools(self):
        """The runner or service the engine needs besides Ollama."""

    def models(self):
        wanted = self.wanted_models()
        return {role: dict(self.ollama["models"][wanted[role]], role=role) for role in self.models_used}

    def own_dir(self):
        path = os.path.join(self.state, self.name)
        shutil.rmtree(path, ignore_errors=True)
        os.makedirs(path)
        return path


class Mem0Adapter(LLMAdapter):
    """mem0 through its Python SDK, which is how it is used now.

    OpenMemory MCP, the server this comparison first meant to drive, was removed
    from the mem0 repository, so the SDK is the supported surface. The provenance
    is a `key` in each memory's metadata, which survives fact extraction.
    """

    PIN = "2.2.1"
    OLLAMA_CLIENT_PIN = "0.6.3"
    infer = False

    def check_tools(self):
        need_tool("uv", "to run the pinned mem0 SDK in an isolated environment")

    def uv_run(self):
        """`uv run` for the pinned SDK, with what to run left to the caller.

        The pins and the interpreter live here and nowhere else, so the version
        probe cannot be silently reshaped by a change to the worker's own
        arguments the way `command()[:-2]` could.
        """
        return ["uv", "run", "--no-project", "--python", "3.12", "--with", f"mem0ai=={self.PIN}",
                "--with", f"ollama=={self.OLLAMA_CLIENT_PIN}"]

    def command(self):
        return self.uv_run() + ["python", "-u", os.path.join(HERE, "llm_workers", "mem0_worker.py")]

    def env(self):
        return isolated_env(self.state, self.name, MEM0_TELEMETRY="False", MEM0_DIR=os.path.join(self.state, "mem0"),
                            **uv_env())

    def version(self):
        r = subprocess.run(self.uv_run() + ["python", "-c", "import importlib.metadata as m; print(m.version('mem0ai'))"],
                           capture_output=True, text=True, env=self.env(), timeout=UV_RUN_SECONDS)
        if r.returncode:
            raise RuntimeError(f"could not import mem0ai=={self.PIN}: {r.stderr.strip()[-300:]}")
        return r.stdout.strip().splitlines()[-1]

    def worker(self):
        if "w" not in self._servers:
            d = self.own_dir()
            w = LineWorker(self.command(), d, self.env(), os.path.join(d, "worker.log"))
            w.send(dict(dir=d, ollama_url=OLLAMA_URL, llm_model=LLM_MODEL, embed_model=EMBED_MODEL),
                   timeout=UV_RUN_SECONDS)
            self._servers["w"] = w
        return self._servers["w"]

    def load(self, members):
        w = self.worker()
        names = stored_titles(members)
        empty, merged = 0, 0
        for m in members:
            out = w.send(dict(op="add", text=f"{names[m['key']]}\n{m['content']}", user=m["project"], key=m["key"],
                              infer=self.infer), timeout=MEM0_ADD_SECONDS)
            events = out["events"]
            if not events:
                empty += 1
                continue
            merged += sum(e in ("UPDATE", "DELETE") for e in events)
            self.keymap[m["key"]] = m["key"]
        # Every row reports stored against sent. Without extraction a write that
        # returned no events is a memory mem0 dropped, and the count says so on
        # the row rather than leaving it to read as an ordinary search miss; with
        # extraction the same count is the memories the model kept a fact from,
        # and the drops are facts it chose not to write.
        stored = len(self.keymap)
        if self.infer:
            self.notes.append(f"{stored} of {len(members)} memories produced at least one stored fact"
                              + (f"; {merged} extraction events updated or deleted an earlier fact" if merged else ""))
        else:
            self.notes.append(f"{stored} of {len(members)} memories were stored"
                              + (f"; {empty} write{'' if empty == 1 else 's'} returned no events" if empty else ""))
        if not self.keymap:
            raise RuntimeError("mem0 stored nothing")

    def search(self, project, query, limit):
        out = self.worker().send(dict(op="search", query=query, user=project, limit=limit), timeout=MEM0_SEARCH_SECONDS)
        return [Hit(h["key"], h["text"]) for h in out["hits"]]


class Mem0VerbatimAdapter(Mem0Adapter):
    name, label, infer = "mem0-verbatim", "mem0 (infer=False)", False
    models_used = ("embed",)
    notes_text = ("stored verbatim, no model call on write; Ollama embeddings only",)


class Mem0InferAdapter(Mem0Adapter):
    name, label, infer = "mem0-infer", "mem0 (infer=True)", True

    def check(self):
        gate(self.name)
        super().check()

    models_used = ("llm", "embed")
    notes_text = ("the LLM extracts the facts that are stored; only those are searchable",)


class ClaudeMemAdapter(LLMAdapter):
    """claude-mem through the MCP server's `save_memory` and `search` tools.

    `save_memory` stores the text as given, so this row calls no model; the engine
    would use one in its hook-driven capture, which needs a live Claude Code
    session and is out of scope here. Its own vector index is a local ONNX
    embedder run by Chroma, not Ollama. The worker the MCP server starts outlives
    it, so `close` stops that worker and its Chroma child.
    """

    name, label = "claude-mem", "claude-mem"
    PIN = "16.1.2"
    models_used = ()
    notes_text = ("measured through `save_memory` and `search`; the write path calls no model, "
                  "and its index is Chroma's local ONNX embedder, not Ollama",)

    def check_tools(self):
        need_tool("npm", "to install the pinned claude-mem package")
        need_tool("node", "to run its MCP server")
        need_tool("bun", "to run its worker, which the MCP server starts")
        need_tool("uvx", "to run its Chroma vector index")

    def package_dir(self):
        return os.path.join(TOOLS, "claude-mem")

    def plugin(self):
        return os.path.join(self.package_dir(), "node_modules", "claude-mem", "plugin")

    def install(self):
        if os.path.isfile(os.path.join(self.plugin(), "scripts", "mcp-server.cjs")):
            return
        os.makedirs(self.package_dir(), exist_ok=True)
        r = subprocess.run(["npm", "install", "--prefix", self.package_dir(), f"claude-mem@{self.PIN}",
                            "--ignore-scripts", "--no-audit", "--no-fund"], capture_output=True, text=True,
                           env=isolated_env(self.state, "npm", **package_cache_env()), timeout=CLAUDE_MEM_INSTALL_SECONDS)
        if r.returncode:
            raise RuntimeError(f"npm install claude-mem@{self.PIN} failed: {(r.stderr or r.stdout).strip()[-300:]}")

    def version(self):
        self.install()
        return json.load(open(os.path.join(self.package_dir(), "node_modules", "claude-mem", "package.json")))["version"]

    def data_dir(self):
        return os.path.join(self.state, "claude-mem-data")

    def server(self):
        if "mcp" not in self._servers:
            shutil.rmtree(self.data_dir(), ignore_errors=True)
            os.makedirs(self.data_dir())
            self.port = free_port()
            env = isolated_env(self.state, "claude-mem", CLAUDE_MEM_DATA_DIR=self.data_dir(),
                               CLAUDE_MEM_WORKER_PORT=str(self.port), CLAUDE_PLUGIN_ROOT=self.plugin(),
                               CLAUDE_MEM_TELEMETRY="off", DO_NOT_TRACK="1", **uv_env())
            cwd = os.path.join(self.state, "work", "claude-mem")
            os.makedirs(cwd, exist_ok=True)
            self._servers["mcp"] = MCP(self.name, "-", launch=(
                ["node", os.path.join(self.plugin(), "scripts", "mcp-server.cjs")], cwd, env))
        return self._servers["mcp"]

    def call(self, tool, args, attempts=4):
        """A tool call, retried while the worker the MCP server just started is
        warming up: the first requests after launch fail with a worker error
        while Chroma downloads its embedder, and the failure is transient."""
        text = ""
        for _ in range(attempts):
            text, _, raw = self.server().call(tool, args)
            if "error" not in raw and not (raw.get("result") or {}).get("isError") \
                    and "could not complete this request" not in text:
                return text
            time.sleep(CLAUDE_MEM_RETRY_SECONDS)
        raise RuntimeError(f"{tool} kept failing: {text[:200]}")

    def load(self, members):
        self.install()
        self.server()
        self.call("search", {"query": "warmup", "limit": 1}, attempts=30)
        names = stored_titles(members)
        newest = {}
        for m in members:
            text = self.call("save_memory", {"text": m["content"], "title": names[m["key"]], "project": m["project"],
                                             "metadata": {"key": m["key"], "type": m["type"]}})
            ident = parse_saved_id(text)
            if ident is None:
                raise RuntimeError(f"save_memory answered something with no memory id: {text[:200]}")
            if ident in self.keymap:
                raise RuntimeError(f"save_memory folded {m['key']} into {self.keymap[ident]}; "
                                   "the store would hold fewer memories than the corpus")
            self.keymap[ident] = m["key"]
            newest[m["project"]] = max(newest.get(m["project"], 0), int(ident))
        self.wait_indexed(newest)

    def wait_indexed(self, newest):
        """The vector index is filled in the background; ask nothing until it holds every memory."""
        path = os.path.join(self.data_dir(), "chroma-sync-state.json")

        def synced():
            state = json.load(open(path))
            return all(state.get(p, {}).get("observations", 0) >= n for p, n in newest.items())

        wait_until(synced, CLAUDE_MEM_INDEX_SECONDS, "claude-mem's vector index")

    def search(self, project, query, limit):
        text = self.call("search", {"query": query, "project": project, "limit": limit})
        return [Hit(i, t) for i, t in parse_search_table(text)]

    def close(self):
        super().close()
        if getattr(self, "port", None) is None:
            return
        self.stop_worker()

    def stop_worker(self):
        """Stop the worker daemon and the Chroma process it started.

        Only what carries this run's data directory or port is matched: another
        claude-mem on the machine, the developer's own, is never touched.
        """
        pidfile = os.path.join(self.data_dir(), "worker.pid")
        try:
            os.kill(int(json.load(open(pidfile))["pid"]), signal.SIGTERM)
        except (OSError, ValueError, KeyError):
            pass
        subprocess.run(["pkill", "-f", "--", self.data_dir()], capture_output=True)


def parse_saved_id(text):
    m = re.search(r"Saved memory #(\d+)", text)
    return m.group(1) if m else None


def parse_search_table(text):
    """`(id, title)` for each row of claude-mem's markdown result table, in rank order."""
    return [(m.group(1), m.group(2).strip()) for m in
            re.finditer(r"^\|\s*#(\d+)\s*\|[^|]*\|[^|]*\|([^|]*)\|", text, re.M)]


def embedding_width(model, url=None):
    """How wide the embedder's vectors are, asked of the model itself."""
    req = urllib.request.Request((url or OLLAMA_URL) + "/api/embed", method="POST",
                                 data=json.dumps({"model": model, "input": "width"}).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=EMBEDDING_SECONDS) as r:
        return len(json.load(r)["embeddings"][0])


def redis_answers(port):
    """FalkorDB speaks the Redis protocol: it is ready when it answers PING."""
    with socket.create_connection(("127.0.0.1", port), timeout=2) as s:
        s.sendall(b"PING\r\n")
        return s.recv(16).startswith(b"+PONG")


def episode_key(refs, keymap):
    """The corpus memory a graph fact comes from, or None.

    A fact lists the episodes it was extracted from, and each episode is a memory
    we wrote under a uuid we chose. A fact drawn from two memories is neither, and
    guessing between them would hand one rank to a memory the engine did not
    separate from the other: it is unmappable, as in the content rule.
    """
    keys = {keymap[r] for r in refs if r in keymap}
    return keys.pop() if len(keys) == 1 else None


class GraphitiAdapter(LLMAdapter):
    """Graphiti's MCP server on a throwaway FalkorDB, with Ollama behind the OpenAI API.

    `add_memory` returns at once and the episode is extracted in the background
    with no completion signal, so each episode is named for its corpus key and the
    loader waits until `get_episodes` lists that name. Search returns entities and facts and
    never the saved text; a fact names the episodes it was drawn from, which is
    how a hit becomes a corpus memory. The model is asked for `json_object`
    output because a small local model does not reliably honour a JSON schema.
    """

    name, label = "graphiti", "Graphiti MCP"
    containers = ()
    IMAGE = "zepai/knowledge-graph-mcp:1.1.0-standalone"
    FALKORDB_IMAGE = "falkordb/falkordb:v4.22.0"
    # No episode finishing for this long means the engine has stopped making
    # progress on the ones left, not that they are slow.
    STALL_SECONDS = 900
    models_used = ("llm", "embed")
    notes_text = ("search returns extracted facts, never the saved text; a fact is mapped to the memory "
                  "its source episode is, and a fact drawn from two memories is unmappable",)

    def check(self):
        gate(self.name)
        super().check()

    def check_tools(self):
        docker_ready()
        # Never pulled here: the image is several gigabytes and a pull on a nearly
        # full disk is how a run took the machine's other containers down.
        for image in (self.IMAGE, self.FALKORDB_IMAGE):
            if subprocess.run(["docker", "image", "inspect", image], capture_output=True).returncode:
                raise Skipped(f"`{image}` is not present and the harness does not pull images; "
                              f"`docker pull {image}` first, with the disk space it needs")

    def version(self):
        r = subprocess.run(["docker", "image", "inspect", self.IMAGE, "--format", "{{.Id}}"],
                           capture_output=True, text=True)
        return f"1.1.0 (`{self.IMAGE}` {r.stdout.strip().replace('sha256:', '')[:12]})"

    def server(self):
        if "mcp" in self._servers:
            return self._servers["mcp"]
        self.containers = []
        name, ports = start_container(self.FALKORDB_IMAGE, "graphiti-falkordb", [6379])
        self.containers.append(name)
        wait_until(lambda: redis_answers(ports[6379]), FALKORDB_READY_SECONDS, "FalkorDB")
        width = embedding_width(EMBED_MODEL)
        host = "host.docker.internal"
        env = {"OPENAI_API_URL": OLLAMA_URL.replace("localhost", host).replace("127.0.0.1", host) + "/v1",
               "OPENAI_API_KEY": "ollama", "MODEL_NAME": LLM_MODEL, "EMBEDDER_MODEL": EMBED_MODEL,
               "LLM_STRUCTURED_OUTPUT_MODE": "json_object", "FALKORDB_URI": f"redis://{host}:{ports[6379]}",
               "SEMAPHORE_LIMIT": "1", "GRAPHITI_TELEMETRY_ENABLED": "false"}
        # The shipped config pins 1536 dimensions, which is OpenAI's embedder and
        # not the local model's; it is rewritten inside the container so that no
        # file is mounted from the host.
        script = (f"sed 's/dimensions: 1536/dimensions: {width}/' config/config.yaml > /tmp/config.yaml && "
                  "exec uv run --no-sync main.py --transport stdio --config /tmp/config.yaml")
        os.makedirs(self.state, exist_ok=True)
        gname = track_container("graphiti-mcp")
        self.containers.append(gname)
        cmd = run_command(self.IMAGE, "graphiti-mcp", [], env, ["sh", "-c", script], interactive=True)
        cmd.insert(cmd.index(self.IMAGE), "--entrypoint=")
        self._servers["mcp"] = MCP(self.name, "-", launch=(cmd, self.state, dict(os.environ)))
        return self._servers["mcp"]

    def load(self, members):
        server = self.server()
        by_project = {}
        for m in members:
            # No uuid is passed: in this release a uuid names an episode that must
            # already exist, and an unknown one is dropped by the queue worker with
            # "node ... not found". The episode's name is ours instead, and
            # `get_episodes` returns the uuid the engine gave it.
            reply, _, _ = call_json(server, "add_memory", {
                "name": m["key"], "episode_body": f"{m['title']}\n{m['content']}", "group_id": m["project"],
                "source": "text", "source_description": f"{m['type']} memory"})
            if "error" in reply:
                raise RuntimeError(f"add_memory refused {m['key']}: {reply['error']}")
            by_project.setdefault(m["project"], set()).add(m["key"])
        lost = 0
        for project, keys in by_project.items():
            lost += len(self.wait_settled(project, keys))
        if lost:
            self.notes.append(f"{lost} of {len(members)} episodes never finished extraction and are not searchable")
        if lost == len(members):
            raise RuntimeError("no episode finished extraction")

    def wait_settled(self, project, keys):
        """The keys of `keys` whose episode never appeared in `get_episodes`.

        An episode is listed once it has been extracted and written, so its
        appearance is the completion signal the engine does not otherwise give.
        """
        server = self.server()
        pending, progress = set(keys), time.time()
        while pending and time.time() - progress < self.STALL_SECONDS:
            reply, _, _ = call_json(server, "get_episodes", {"group_ids": [project], "max_episodes": 1000})
            seen = {e["name"]: e["uuid"] for e in reply.get("episodes") or [] if e.get("name") in pending}
            for name, ident in seen.items():
                self.keymap[ident] = name
            if seen:
                pending -= set(seen)
                progress = time.time()
            else:
                time.sleep(5)
        return pending

    def search(self, project, query, limit):
        reply, _, _ = call_json(self.server(), "search_memory_facts",
                                {"query": query, "group_ids": [project], "max_facts": limit})
        if "error" in reply:
            raise RuntimeError(reply["error"])
        return [Hit(",".join(f.get("episodes") or []) or "-", f.get("fact")) for f in reply.get("facts") or []]

    def map_hit(self, hit, members):
        return episode_key([r for r in (hit.ref or "").split(",") if r], self.keymap)

    def close(self):
        super().close()
        for name in self.containers:
            remove_container(name)
        self.containers = []


class LettaAdapter(LLMAdapter):
    """Letta, which cannot be measured as the research described it.

    The plan was Letta 0.34.10 as an HTTP server with archival passages inserted
    and searched through `letta-client`, on Postgres with pgvector. That server no
    longer exists at that version. `letta` 0.34.10 on PyPI is Letta Code, a
    terminal agent: installed into a throwaway environment it prints Letta Code's
    usage and has no archival-memory API. The `letta/letta:0.34.10` image is the
    same program (its command is `letta --help` and it exposes no port), and the
    repository's README says the Letta V1 API server was retired to an `archive`
    branch. The last server release is 0.16.8, whose image does listen on 8283 and
    ship Postgres, but that is a different, retired product from the version the
    comparison was to name, and it was not run. The row is therefore a skip with
    this reason, which is what the brief asked for rather than a substitute.
    """

    name, label = "letta", "Letta"
    models_used = ()

    def check(self):
        raise Skipped(
            "Letta 0.34.10 has no archival-memory server: the PyPI package and the `letta/letta:0.34.10` image are "
            "Letta Code, a terminal agent, and the V1 API server with archival passages was retired after 0.16.8, "
            "which is a different product from the version named and was not measured")


LLM = (Mem0VerbatimAdapter, Mem0InferAdapter, ClaudeMemAdapter, GraphitiAdapter, LettaAdapter)
