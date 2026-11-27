#!/usr/bin/env python3
"""What an engine puts in the model's context over a session, and whether the
per-prompt hint speaks when it should.

Two modes, one file, because both run the shipped hooks against a copy of a
store and read what the model would read:

  Session model (default). For each engine: the session-start hook, the
  per-prompt hook over a list of prompts, the post-compaction hook, and the
  replies of mem_context, mem_save and mem_search, plus the `tools/list` input
  schemas and the server instructions. Totals are for one 20-prompt session in
  two shapes -- "upfront", where every schema is paid on every session, and
  "deferred", where the client loads schemas on demand and they cost nothing
  until used.

  Labels (--labels TSV). Replays the per-prompt hook once per hand-labelled
  prompt, in a fresh session each time, and scores it: silent on a prompt that
  needs nothing, naming a memory the labeller marked as the right one on a
  prompt that has one, the precision of every line named, and the tokens and
  bytes the hint cost.

Nothing here touches the store it is given. Every store is copied into a
temporary directory with SQLite's backup API first and every engine runs with
HOME, its data directory and its Claude config pointing inside that directory.
The user's own stores are never opened for writing, and a run cannot find them
by accident because the variables that name them are removed from the
environment before the isolated ones are set.

    python3 tools/memory-bench/session_cost.py \\
        --leteo-bin target/release/leteo --leteo-store copy-of-store.db \\
        --engram-bin $(command -v engram) --engram-store engram-copy.db \\
        --engram-plugin-root path/to/engram/plugin/claude-code \\
        --project my-project

Use a Python that has `tiktoken` for token counts. Without it the figures are
bytes divided by four and say so in the output, which is a different unit and
must not be set beside a tiktoken run.
"""

import argparse
import importlib
import json
import math
import os
import pathlib
import re
import socket
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid

HERE = pathlib.Path(__file__).resolve().parent
DATA = HERE / "session_cost_data"

# One 20-prompt session's tool calls, the same for both engines.
MIX = {"mem_context": 1, "mem_search": 6, "mem_save": 3}
# Engram's save nudge fires at most once per 15 minutes once the session is five
# minutes old; a 20-prompt session is assumed to see it once.
ENGRAM_NUDGES = 1

# Variables that can name a store or an endpoint. Removed from the inherited
# environment before the isolated ones are set, so a developer who exports one
# for daily use does not point a run at their own memory.
SCRUBBED = (
    "ENGRAM_DATA_DIR", "ENGRAM_PORT", "ENGRAM_URL", "ENGRAM_SOCKET", "ENGRAM_PROJECT",
    "ENGRAM_CLOUD_TOKEN", "ENGRAM_CLOUD_SERVER", "ENGRAM_CLOUD_AUTOSYNC",
    "LETEO_DATA_DIR", "LETEO_DATABASE", "LETEO_MODEL_DIR", "LETEO_TRACE_RECALL",
    "CLAUDE_CONFIG_DIR", "CLAUDE_PLUGIN_ROOT",
)


# --- counting ---------------------------------------------------------------

def make_counter():
    """(count, unit). tiktoken's cl100k_base when installed, else bytes/4."""
    try:
        import tiktoken
        enc = tiktoken.get_encoding("cl100k_base")
        return (lambda text: len(enc.encode(text or ""))), "tokens (tiktoken cl100k_base)"
    except Exception:
        return (lambda text: math.ceil(len((text or "").encode()) / 4)), "~tokens (bytes/4, tiktoken not available)"


# --- what the model reads ---------------------------------------------------

def visible_hook_text(stdout):
    """What Claude Code puts in the model's context from a hook's stdout.

    JSON output contributes hookSpecificOutput.additionalContext only
    (systemMessage is rendered to the user, not the model); plain-text stdout
    from SessionStart / UserPromptSubmit is added verbatim.
    """
    s = stdout.strip()
    if not s:
        return ""
    try:
        obj = json.loads(s)
    except ValueError:
        return stdout
    if not isinstance(obj, dict):
        return stdout
    return (obj.get("hookSpecificOutput") or {}).get("additionalContext") or ""


def call_text(resp):
    """The text blocks of a tools/call result: what the model reads."""
    res = resp.get("result") or {}
    parts = [c.get("text", "") for c in res.get("content", []) if c.get("type") == "text"]
    if not parts and "error" in resp:
        parts = [json.dumps(resp["error"])]
    return "\n".join(parts)


def model_visible_tool(tool):
    """A tool as the model sees it in its tool list.

    `outputSchema` is left out: a client sends the model a tool's name,
    description and input schema, and a reply's structured content is not
    described to it by the output schema. Counting it overstated Leteo's fixed
    cost by several times, because the output schemas are most of the bytes in
    `tools/list`.
    """
    return {"name": tool["name"], "description": tool.get("description", ""),
            "inputSchema": tool.get("inputSchema", {})}


def p90(xs):
    s = sorted(xs)
    return s[math.ceil(0.9 * len(s)) - 1]


def read_lines(path):
    return [l for l in pathlib.Path(path).read_text().splitlines() if l.strip()]


# --- isolation --------------------------------------------------------------

def copy_store(src, dst):
    """Copy `src` to `dst` with the backup API. `src` is opened read-only.

    A plain file copy of a store in WAL mode loses whatever is still in the
    -wal file, and copying it while another process writes can tear it. The
    backup API reads one consistent snapshot, and `mode=ro` means this tool
    could not write the original even by mistake.
    """
    src = pathlib.Path(src).expanduser().resolve()
    if not src.is_file():
        sys.exit(f"store not found: {src}")
    try:
        ro = sqlite3.connect(f"file:{src}?mode=ro", uri=True)
        out = sqlite3.connect(dst)
        with out:
            ro.backup(out)
        out.close()
        ro.close()
    except sqlite3.Error as e:
        sys.exit(f"could not copy {src} read-only: {e}. A WAL store whose -shm file is missing "
                 "cannot be opened read-only; copy it with `sqlite3 FILE \".backup COPY\"` first.")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def base_env(home, extra_path=()):
    env = {k: v for k, v in os.environ.items() if k not in SCRUBBED}
    env["HOME"] = str(home)
    env["PATH"] = os.pathsep.join([*map(str, extra_path), env.get("PATH", "")])
    return env


class Engine:
    """One engine over its own throwaway copy of a store."""

    def __init__(self, name, binary, store, work, project, plugin_root=None, model_dir=None, settings=None):
        self.name = name
        self.bin = str(pathlib.Path(binary).resolve()) if os.sep in str(binary) else str(binary)
        self.project = project
        self.home = work / f"home-{name}"
        self.data = self.home / f".{name}"
        self.data.mkdir(parents=True)
        (self.home / ".claude").mkdir()
        # A directory named for the project, because both engines detect the
        # project from the working directory.
        # Short on purpose: the working directory is echoed in some replies
        # (Engram's search results, Leteo's save acknowledgement), so its length
        # is part of what is counted. A system temp directory under macOS is
        # three times as long as a developer's checkout path.
        self.cwd = work / name[0] / project
        self.cwd.mkdir(parents=True)
        self.plugin_root = pathlib.Path(plugin_root).resolve() if plugin_root else None
        self.server = None
        self.failures = []
        bindir = pathlib.Path(self.bin).parent if os.sep in self.bin else None
        env = base_env(self.home, [bindir] if bindir else [])
        env["CLAUDE_CONFIG_DIR"] = str(self.home / ".claude")
        if name == "leteo":
            self.db = self.data / "leteo.db"
            env["LETEO_DATA_DIR"] = str(self.data)
            env["LETEO_DATABASE"] = str(self.db)
            if model_dir:
                env["LETEO_MODEL_DIR"] = str(pathlib.Path(model_dir).resolve())
            if settings:
                (self.data / "settings.json").write_text(settings)
        else:
            self.db = self.data / "engram.db"
            self.port = free_port()
            env["ENGRAM_DATA_DIR"] = str(self.data)
            env["ENGRAM_PORT"] = str(self.port)
            env["ENGRAM_NO_UPDATE_CHECK"] = "1"
            if self.plugin_root:
                env["CLAUDE_PLUGIN_ROOT"] = str(self.plugin_root)
        self.env = env
        copy_store(store, self.db)

    def start(self):
        if self.name != "engram":
            return
        # Engram's hooks are shell scripts that talk to a running server.
        self.server = subprocess.Popen([self.bin, "serve", str(self.port)], env=self.env, cwd=self.cwd,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        url = f"http://127.0.0.1:{self.port}/health"
        for _ in range(100):
            try:
                urllib.request.urlopen(url, timeout=1).read()
                return
            except Exception:
                time.sleep(0.1)
        sys.exit("engram serve did not come up on its private port")

    def stop(self):
        if self.server:
            self.server.terminate()
            self.server.wait()

    def hook_cmd(self, kind):
        if self.name == "leteo":
            sub = {"start": "session-start", "compact": "post-compaction", "prompt": "user-prompt-submit"}[kind]
            return [self.bin, "hook", sub]
        if not self.plugin_root:
            sys.exit("--engram-plugin-root is required to run Engram's hooks (the scripts live in its plugin, not its binary)")
        script = {"start": "session-start.sh", "compact": "post-compaction.sh", "prompt": "user-prompt-submit.sh"}[kind]
        return ["bash", str(self.plugin_root / "scripts" / script)]

    def hook(self, kind, session_id, **extra):
        body = {"session_id": session_id, "transcript_path": str(self.data / "transcript.jsonl"),
                "cwd": str(self.cwd), "hook_event_name": "UserPromptSubmit" if kind == "prompt" else "SessionStart"}
        body.update(extra)
        t0 = time.perf_counter()
        try:
            r = subprocess.run(self.hook_cmd(kind), input=json.dumps(body), capture_output=True, text=True,
                               cwd=self.cwd, env=self.env, timeout=120)
            out, err, rc = r.stdout, r.stderr, r.returncode
        except subprocess.TimeoutExpired:
            out, err, rc = "", "timed out after 120 s", -1
        if rc != 0:
            # Empty stdout from a crashed hook reads as "said nothing", which is
            # a measurement of a quiet hook, not of a broken one. Recorded here,
            # where every call passes, so no mode can forget to look.
            first = next((l for l in err.splitlines() if l.strip()), "no stderr")
            what = f"{kind} hook" + (f" on prompt {extra['prompt']!r}" if "prompt" in extra else "")
            self.failures.append(f"{self.name}: {what} exited {rc}: {first}")
        return out, err, rc, (time.perf_counter() - t0) * 1000

    def refuse_failed_measurement(self):
        """Exit non-zero, naming each failed hook, if any hook did not exit cleanly."""
        if self.failures:
            sys.exit("measurement failed, no result printed:\n  " + "\n  ".join(self.failures))

    def mcp_cmd(self):
        return [self.bin, "mcp", "--tools=agent"]


class Mcp:
    def __init__(self, cmd, cwd, env):
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                  text=True, cwd=cwd, env=env, bufsize=1)
        self.next_id = 1

    def request(self, method, params=None):
        rid = self.next_id
        self.next_id += 1
        msg = {"jsonrpc": "2.0", "id": rid, "method": method}
        if params is not None:
            msg["params"] = params
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("MCP server closed its output")
            obj = json.loads(line)
            if obj.get("id") == rid:
                return obj

    def notify(self, method):
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method}) + "\n")
        self.p.stdin.flush()

    def close(self):
        self.p.stdin.close()
        try:
            self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.p.kill()


# --- session model ----------------------------------------------------------

def hooks_measure(eng, prompts, count, raw):
    res = {}
    sid = str(uuid.uuid4())
    so, se, rc, _ = eng.hook("start", sid, source="startup", model="claude-opus-5-5")
    vis = visible_hook_text(so)
    raw["session_start"] = vis
    res["session_start"] = count(vis)
    res["session_start_rc"] = rc

    per, ms, rows = [], [], []
    for p in prompts:
        so, se, rc, t = eng.hook("prompt", sid, prompt=p, permission_mode="default")
        vis = visible_hook_text(so)
        per.append(count(vis))
        ms.append(t)
        rows.append({"prompt": p, "rc": rc, "tokens": count(vis), "visible": vis})
    raw["prompts"] = rows
    res["prompts"] = per
    res["prompt_ms_median"] = round(statistics.median(ms)) if ms else 0

    so, se, rc, _ = eng.hook("compact", sid, source="compact", model="claude-opus-5-5")
    vis = visible_hook_text(so)
    raw["post_compaction"] = vis
    res["post_compaction"] = count(vis)
    res["session_id"] = sid

    if eng.name == "engram":
        res["nudge"] = engram_nudge(eng, sid, count, raw)
    return res


def engram_nudge(eng, sid, count, raw):
    """Trigger Engram's debounced save nudge once and count it.

    The nudge needs a session more than five minutes old, so the measured
    session is backdated by ten minutes in the throwaway copy and one more
    prompt is submitted.
    """
    c = sqlite3.connect(eng.db)
    c.execute("update sessions set started_at=datetime('now','-10 minutes') where id=?", (sid,))
    c.commit()
    c.close()
    so, _, _, _ = eng.hook("prompt", sid, prompt="ok, keep going", permission_mode="default")
    vis = visible_hook_text(so)
    raw["nudge"] = vis
    return count(vis)


def mcp_measure(eng, queries, saves, count, raw):
    m = Mcp(eng.mcp_cmd(), eng.cwd, eng.env)
    init = m.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                    "clientInfo": {"name": "claude-code", "version": "2.1.0"}})
    m.notify("notifications/initialized")
    instructions = (init.get("result") or {}).get("instructions") or ""
    tools = m.request("tools/list", {})["result"]["tools"]
    per_tool = {t["name"]: count(json.dumps(model_visible_tool(t), ensure_ascii=False)) for t in tools}
    # Reported so the correction is checkable: these bytes are in `tools/list`
    # and the model is not sent them.
    output_schema = sum(count(json.dumps(t["outputSchema"], ensure_ascii=False)) for t in tools if "outputSchema" in t)
    res = {"instructions": count(instructions), "tool_count": len(tools),
           "schema": sum(per_tool.values()), "output_schema_not_visible": output_schema,
           "per_tool": per_tool}

    txt = call_text(m.request("tools/call", {"name": "mem_context", "arguments": {}}))
    raw["mem_context"] = txt
    res["mem_context"] = count(txt)

    search = []
    raw["mem_search"] = []
    for q in queries:
        txt = call_text(m.request("tools/call", {"name": "mem_search", "arguments": {"query": q}}))
        search.append(count(txt))
        raw["mem_search"].append({"query": q, "tokens": count(txt), "text": txt})
    res["mem_search"] = search

    out = []
    raw["mem_save"] = []
    for s in saves:
        args = dict(s, project=eng.project)
        txt = call_text(m.request("tools/call", {"name": "mem_save", "arguments": args}))
        out.append(count(txt))
        raw["mem_save"].append({"title": s["title"], "tokens": count(txt), "text": txt})
    res["mem_save"] = out
    m.close()
    return res


def session_row(r, engram):
    """One engine's table row and its session totals from the raw measurement."""
    h, m = r["hooks"], r["mcp"]
    prompts = h["prompts"]
    ups = sum(prompts) + (ENGRAM_NUDGES * h.get("nudge", 0) if engram else 0)
    search_mean = statistics.mean(m["mem_search"])
    save_med = statistics.median(m["mem_save"])
    fixed = m["schema"] + m["instructions"]
    mix = MIX["mem_context"] * m["mem_context"] + MIX["mem_search"] * search_mean + MIX["mem_save"] * save_med
    deferred = h["session_start"] + ups + h["post_compaction"] + mix
    return {
        "session_start": h["session_start"],
        "prompt_hook_session": ups,
        "prompt_hook_injected": sum(1 for x in prompts if x),
        "prompt_hook_prompts": len(prompts),
        "prompt_hook_nudge": h.get("nudge", 0) if engram else 0,
        "post_compaction": h["post_compaction"],
        "mem_context": m["mem_context"],
        "mem_save_median": save_med,
        "mem_search_mean": round(search_mean, 1),
        "mem_search_p90": p90(m["mem_search"]),
        "tools": m["tool_count"],
        "schema": m["schema"],
        "output_schema_not_visible": m["output_schema_not_visible"],
        "instructions": m["instructions"],
        "fixed": fixed,
        "mix": round(mix, 1),
        "total_upfront": round(deferred + fixed),
        "total_deferred": round(deferred),
    }


ROWS = (
    ("session start", "session_start"),
    ("per-prompt hook, whole session", "prompt_hook_session"),
    ("post-compaction", "post_compaction"),
    ("mem_context", "mem_context"),
    ("mem_save (median)", "mem_save_median"),
    ("mem_search (mean)", "mem_search_mean"),
    ("tools/list (input schemas)", "schema"),
    ("server instructions", "instructions"),
    ("session total, upfront", "total_upfront"),
    ("session total, deferred", "total_deferred"),
)


def print_session_table(rows, unit):
    names = list(rows)
    print(f"\nUnit: {unit}. Mix per session: {MIX['mem_context']} mem_context, "
          f"{MIX['mem_search']} mem_search, {MIX['mem_save']} mem_save.\n")
    print(f"{'':34}" + "".join(f"{n:>12}" for n in names) + (f"{'delta':>9}" if len(names) == 2 else ""))
    for label, key in ROWS:
        vals = [rows[n][key] for n in names]
        line = f"{label:34}" + "".join(f"{v:>12,.0f}" if key != "mem_search_mean" else f"{v:>12,.1f}" for v in vals)
        if len(names) == 2 and vals[1]:
            line += f"{100 * (vals[0] / vals[1] - 1):>+8.1f}%"
        print(line)
    for n in names:
        r = rows[n]
        print(f"{n}: {r['tools']} tools; per-prompt hook spoke on {r['prompt_hook_injected']}/{r['prompt_hook_prompts']} prompts"
              + (f" (+{r['prompt_hook_nudge']} for one save nudge)" if r["prompt_hook_nudge"] else "")
              + f"; outputSchema, not model-visible: {r['output_schema_not_visible']:,}")
    if len(names) == 2:
        print(f"delta is {names[0]} against {names[1]}.")


def run_session(args, count, unit, work):
    prompts = read_lines(args.prompts)
    queries = read_lines(args.queries)
    saves = json.loads(pathlib.Path(args.saves).read_text())
    engines = []
    if args.leteo_store:
        engines.append(Engine("leteo", args.leteo_bin, args.leteo_store, work, args.project,
                              model_dir=args.model_dir, settings=args.leteo_settings))
    if args.engram_store:
        engines.append(Engine("engram", args.engram_bin, args.engram_store, work, args.project,
                              plugin_root=args.engram_plugin_root))
    if not engines:
        sys.exit("give at least one of --leteo-store and --engram-store")
    rows, versions = {}, {}
    raws = {}
    for eng in engines:
        versions[eng.name] = subprocess.run([eng.bin, "version" if eng.name == "engram" else "--version"],
                                            capture_output=True, text=True, env=eng.env).stdout.strip()
        raw = raws[eng.name] = {}
        eng.start()
        try:
            hooks = hooks_measure(eng, prompts, count, raw)
            eng.refuse_failed_measurement()
            r = {"hooks": hooks,
                 "mcp": mcp_measure(eng, queries, saves, count, raw)}
        finally:
            eng.stop()
        rows[eng.name] = session_row(r, eng.name == "engram")
        rows[eng.name]["prompt_tokens"] = r["hooks"]["prompts"]
        rows[eng.name]["prompt_ms_median"] = r["hooks"]["prompt_ms_median"]
        rows[eng.name]["per_tool"] = r["mcp"]["per_tool"]
    print_session_table(rows, unit)
    print("versions: " + "; ".join(f"{k}: {v}" for k, v in versions.items()))
    if args.json:
        pathlib.Path(args.json).write_text(json.dumps(
            {"unit": unit, "versions": versions, "mix": MIX, "rows": rows}, indent=2, ensure_ascii=False))
    if args.raw_dir:
        d = pathlib.Path(args.raw_dir)
        d.mkdir(parents=True, exist_ok=True)
        for name, raw in raws.items():
            (d / f"{name}.json").write_text(json.dumps(raw, indent=2, ensure_ascii=False))
    return rows


# --- labels -----------------------------------------------------------------

def unescape(s):
    return s.replace("\\n", "\n").replace("\\t", "\t").replace("\\\\", "\\")


def read_labels(path):
    """Items from a TSV: id, created_at, project, source, prompt, label, best_memory_ids, ...

    Only the first seven columns are read, so a file with the analysis columns a
    labelling pass adds is accepted as it is. `source` `real` means the store is
    cut back to how it stood at `created_at` before the prompt; any other value
    replays against the store as it is.
    """
    lines = pathlib.Path(path).read_text().splitlines()
    out = []
    for line in lines[1:]:
        if not line.strip():
            continue
        c = line.split("\t")
        if len(c) < 7:
            sys.exit(f"{path}: a row has {len(c)} columns, expected at least 7: {line[:60]!r}")
        if c[5] not in ("neutral", "related"):
            sys.exit(f"{path}: label must be neutral or related, got {c[5]!r}")
        out.append({"id": c[0], "created_at": c[1], "project": c[2], "source": c[3], "prompt": unescape(c[4]),
                    "label": c[5], "best": [int(x) for x in c[6].split(",") if x.strip()]})
    return out


def cut_store_to(db, t):
    """Delete from the working copy everything that did not exist at time `t`."""
    c = sqlite3.connect(db)
    c.execute("PRAGMA foreign_keys=OFF")
    c.execute("DELETE FROM observations WHERE created_at >= ?", (t,))
    c.execute("UPDATE observations SET deleted_at=NULL WHERE deleted_at IS NOT NULL AND deleted_at >= ?", (t,))
    c.execute("DELETE FROM memory_relations WHERE created_at >= ? OR updated_at >= ?", (t, t))
    c.commit()
    c.close()


NAMED = re.compile(r"^- #(\d+) ", re.M)


def replay(eng, items, count):
    """Run the prompt hook once per item, a fresh session each time.

    Time-cut items are walked newest first over the one working copy, because
    cutting only removes: going forward in time would need the rows back.
    """
    timecut = sorted((i for i in items if i["source"] == "real"), key=lambda i: (i["created_at"], i["id"]), reverse=True)
    static = [i for i in items if i["source"] != "real"]
    res = []
    for it in static + timecut:
        if it["source"] == "real":
            cut_store_to(eng.db, it["created_at"])
        eng.cwd = eng.cwd.parent / it["project"]
        eng.cwd.mkdir(parents=True, exist_ok=True)
        so, se, rc, ms = eng.hook("prompt", str(uuid.uuid4()), prompt=it["prompt"], permission_mode="default")
        vis = visible_hook_text(so)
        res.append({**{k: it[k] for k in ("id", "label", "best", "source")}, "rc": rc,
                    "injected": [int(x) for x in NAMED.findall(vis)], "tokens": count(vis),
                    "bytes": len(vis.encode("utf-8")), "ms": ms, "visible": vis})
    return res


def score(res):
    neutral = [r for r in res if r["label"] == "neutral"]
    related = [r for r in res if r["label"] == "related"]
    ms = sorted(r["ms"] for r in res)
    named = sum(len(r["injected"]) for r in res)
    named_right = sum(len(set(r["injected"]) & set(r["best"])) for r in res)
    return {
        "neutral": len(neutral),
        "neutral_silent": sum(not r["injected"] for r in neutral),
        "neutral_spoke": sum(bool(r["injected"]) for r in neutral),
        "related": len(related),
        "related_named": sum(bool(set(r["injected"]) & set(r["best"])) for r in related),
        "related_spoke_wrong": sum(bool(r["injected"]) and not set(r["injected"]) & set(r["best"]) for r in related),
        "named": named,
        "named_right": named_right,
        "precision": round(named_right / named, 3) if named else 0.0,
        "tokens_neutral": sum(r["tokens"] for r in neutral),
        "tokens_total": sum(r["tokens"] for r in res),
        "bytes_neutral": sum(r["bytes"] for r in neutral),
        "bytes_total": sum(r["bytes"] for r in res),
        "hook_failures": sum(r["rc"] != 0 for r in res),
        "ms_median": round(statistics.median(ms)) if ms else 0,
        "ms_p90": round(ms[min(len(ms) - 1, int(0.9 * len(ms)))]) if ms else 0,
    }


def run_labels(args, count, unit, work):
    if not args.leteo_store:
        sys.exit("--labels needs --leteo-store: the hint under evaluation is Leteo's")
    items = read_labels(args.labels)
    if args.only:
        items = [i for i in items if (i["source"] == "real") == (args.only == "real")]
    eng = Engine("leteo", args.leteo_bin, args.leteo_store, work, args.project,
                 model_dir=args.model_dir, settings=args.leteo_settings)
    res = replay(eng, items, count)
    eng.refuse_failed_measurement()
    s = score(res)
    print(f"\nUnit: {unit}. {len(items)} labelled prompts, store copy of {args.leteo_store}\n")
    print(f"silent on neutral prompts : {s['neutral_silent']}/{s['neutral']}"
          f" (spoke on {s['neutral_spoke']}, a false-speak rate of {s['neutral_spoke']}/{s['neutral']})")
    print(f"named a right memory      : {s['related_named']}/{s['related']} related prompts"
          f" ({s['related_spoke_wrong']} spoke and named a wrong one)")
    print(f"precision                 : {s['precision']} ({s['named_right']} of {s['named']} named)"
          f", of which {s['named'] - s['named_right']} were unrelated")
    print(f"tokens                    : {s['tokens_total']} total, {s['tokens_neutral']} on neutral prompts")
    print(f"bytes                     : {s['bytes_total']} total, {s['bytes_neutral']} on neutral prompts")
    print(f"hook latency              : median {s['ms_median']} ms, p90 {s['ms_p90']} ms"
          f"; non-zero exits: {s['hook_failures']}")
    if args.json:
        pathlib.Path(args.json).write_text(json.dumps({"unit": unit, "score": s, "items": res}, indent=2, ensure_ascii=False))
    return s


# --- fixture ----------------------------------------------------------------

def build_fixture(args, work):
    """Write `--fixture-corpus`'s corpus to an empty Leteo store through mem_save.

    Memory ids are assigned in write order, so `labels.example.tsv` can name
    them, and a test holds the file's titles to the ids this produces. Timestamps
    are then stamped to fixed values, one hour apart, so the time-cut rows of the
    example labels cut the store at a known place.
    """
    sys.path.insert(0, str(HERE))
    corpus = importlib.import_module(args.fixture_corpus.replace("-", "_") + "_corpus").corpus()
    eng = Engine("leteo", args.leteo_bin, args.build_fixture_seed or empty_store(work, args.leteo_bin), work, corpus[0]["project"])
    servers = {}
    for mem in corpus:
        project = mem["project"]
        if project not in servers:
            # Project detection is by working directory, and a project the store
            # does not hold yet is refused from anywhere else.
            cwd = work / "f" / project
            cwd.mkdir(parents=True, exist_ok=True)
            m = servers[project] = Mcp(eng.mcp_cmd(), cwd, eng.env)
            m.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                     "clientInfo": {"name": "fixture", "version": "0"}})
            m.notify("notifications/initialized")
        call = {"title": mem["title"], "content": mem["content"], "type": mem["type"], "project": project}
        if mem.get("topic"):
            call["topic_key"] = mem["topic"]
        reply = call_text(servers[project].request("tools/call", {"name": "mem_save", "arguments": call}))
        if "error" in reply[:200].lower():
            sys.exit(f"mem_save refused {mem['key']}: {reply[:200]}")
    for m in servers.values():
        m.close()
    c = sqlite3.connect(eng.db)
    rows = c.execute("select id, title from observations order by id").fetchall()
    if [t for _, t in rows] != [m["title"] for m in corpus]:
        sys.exit("the store does not hold the corpus in write order; ids would not match the labels")
    for (i, _) in rows:
        stamp = f"2026-01-{1 + i // 24:02d} {i % 24:02d}:00:00"
        c.execute("update observations set created_at=?, updated_at=? where id=?", (stamp, stamp, i))
    c.commit()
    c.close()
    dst = pathlib.Path(args.build_fixture).resolve()
    copy_store(eng.db, dst)
    print(f"wrote {len(rows)} memories to {dst}")


def empty_store(work, binary):
    """An empty store, made by letting the binary create one."""
    seed = work / "seed.db"
    home = work / "seed-home"
    home.mkdir()
    env = base_env(home)
    env["LETEO_DATA_DIR"] = str(home)
    env["LETEO_DATABASE"] = str(seed)
    r = subprocess.run([binary, "hook", "session-start"], input="{}", capture_output=True, text=True, env=env, cwd=work)
    if not seed.exists():
        sys.exit(f"could not create an empty store with {binary}: {r.stderr.strip()[:200]}")
    return seed


# --- command line -----------------------------------------------------------

def parser():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--leteo-bin", default=os.environ.get("LETEO_BIN", "leteo"))
    p.add_argument("--leteo-store", help="a Leteo database; copied, never opened for writing")
    p.add_argument("--engram-bin", default=os.environ.get("ENGRAM_BIN", "engram"))
    p.add_argument("--engram-store", help="an Engram database; copied, never opened for writing")
    p.add_argument("--engram-plugin-root", default=os.environ.get("ENGRAM_PLUGIN_ROOT"),
                   help="Engram's plugin/claude-code directory, which holds the hook scripts")
    p.add_argument("--project", default="alpha-api", help="the project the working directory names")
    p.add_argument("--prompts", default=str(DATA / "prompts.txt"), help="one prompt per line")
    p.add_argument("--queries", default=str(DATA / "queries.txt"), help="one mem_search query per line")
    p.add_argument("--saves", default=str(DATA / "saves.json"), help="JSON list of mem_save arguments")
    p.add_argument("--model-dir", help="LETEO_MODEL_DIR for the run; by default there is none and recall is lexical")
    p.add_argument("--leteo-settings", help="contents of the isolated settings.json, as a JSON string")
    p.add_argument("--labels", help="a labels TSV: replay the prompt hook over it instead of the session model")
    p.add_argument("--only", choices=("real", "synthetic"),
                   help="with --labels, replay only rows of that source: a labelling pass over a real store and one over "
                        "an imported copy are separate stores, and each wants its own run")
    p.add_argument("--workdir", help="where the throwaway copies and working directories go; a short path keeps the "
                                     "working-directory path that some replies echo close to a real checkout's")
    p.add_argument("--json", help="write the result as JSON")
    p.add_argument("--raw-dir", help="write each engine's captured model-visible text here")
    p.add_argument("--build-fixture", metavar="OUT.db",
                   help="write the synthetic fixture store to OUT.db with --leteo-bin and exit")
    p.add_argument("--fixture-corpus", choices=("comparison", "hint"), default="comparison",
                   help="which corpus --build-fixture writes: the held-out comparison one (labels.example.tsv) "
                        "or the bilingual store the hint is labelled against (labels.hint.tsv)")
    p.add_argument("--build-fixture-seed", help=argparse.SUPPRESS)
    return p


def main(argv=None):
    args = parser().parse_args(argv)
    # Engines run from a directory under the temporary one, so a relative path
    # to a binary would stop resolving the moment it is spawned.
    for name in ("leteo_bin", "engram_bin"):
        value = getattr(args, name)
        if os.sep in value:
            setattr(args, name, str(pathlib.Path(value).resolve()))
    count, unit = make_counter()
    with tempfile.TemporaryDirectory(prefix="sc-", dir=args.workdir) as w:
        work = pathlib.Path(w)
        if args.build_fixture:
            return build_fixture(args, work)
        if args.labels:
            return run_labels(args, count, unit, work)
        return run_session(args, count, unit, work)


if __name__ == "__main__":
    main()
